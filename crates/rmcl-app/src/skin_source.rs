//! Validated skin sources from local files or image URLs.
//!
//! Official Java uploads only accept 64×64 or legacy 64×32 PNGs. Sources from
//! skin sites are downloaded by RMCL first so that the preview, validation and
//! uploaded bytes always describe the same image.

use std::{
    io::Cursor,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::{Path, PathBuf},
    time::Duration,
};

use image::{DynamicImage, ImageFormat, RgbaImage};
use reqwest::{
    StatusCode,
    header::{ACCEPT, CONTENT_TYPE, LOCATION},
};
use rmcl_auth::SkinVariant;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::skins::{SkinError, SkinTexture};

/// Maximum bytes accepted from a file or URL before decoding.
pub const MAX_SKIN_SOURCE_BYTES: usize = 1024 * 1024;
/// Official Java uploads have historically enforced a 24 KiB PNG limit.
pub const MAX_SKIN_UPLOAD_BYTES: usize = 24 * 1024;
const MAX_REDIRECTS: usize = 3;

/// A skin that passed local validation and is ready to preview or upload.
#[derive(Debug, Clone)]
pub struct PreparedSkin {
    id: String,
    bytes: Vec<u8>,
    texture: SkinTexture,
    variant_hint: SkinVariant,
    source_url: Option<String>,
    file_name: String,
}

impl PreparedSkin {
    /// Stable content id used by the local library and preview cache.
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn texture(&self) -> &SkinTexture {
        &self.texture
    }

    /// A best-effort model hint. Callers must still let the user choose.
    pub fn variant_hint(&self) -> SkinVariant {
        self.variant_hint
    }

    pub fn source_url(&self) -> Option<&str> {
        self.source_url.as_deref()
    }

    pub fn file_name(&self) -> &str {
        &self.file_name
    }
}

/// Validates a PNG and normalizes it for the official upload endpoint.
pub fn prepare_skin(
    bytes: &[u8],
    file_name: impl Into<String>,
    source_url: Option<String>,
) -> Result<PreparedSkin, SkinSourceError> {
    if bytes.len() > MAX_SKIN_SOURCE_BYTES {
        return Err(SkinSourceError::SourceTooLarge);
    }
    let texture = SkinTexture::decode(bytes)?;
    let (width, height) = texture.dimensions();
    if !matches!((width, height), (64, 64) | (64, 32)) {
        return Err(SkinSourceError::UnsupportedUploadDimensions { width, height });
    }

    let mut upload_bytes = bytes.to_vec();
    if upload_bytes.len() > MAX_SKIN_UPLOAD_BYTES {
        upload_bytes = encode_png(texture.image())?;
        if upload_bytes.len() > MAX_SKIN_UPLOAD_BYTES {
            return Err(SkinSourceError::UploadTooLarge);
        }
    }

    let id = content_id(&upload_bytes);
    Ok(PreparedSkin {
        id,
        bytes: upload_bytes,
        variant_hint: detect_variant(texture.image()),
        texture,
        source_url,
        file_name: normalized_file_name(&file_name.into()),
    })
}

fn content_id(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    hex::encode(digest.finalize())
}

/// Reads a local PNG and prepares it for upload.
pub async fn load_skin_file(path: impl AsRef<Path>) -> Result<PreparedSkin, SkinSourceError> {
    let path = path.as_ref().to_path_buf();
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|source| SkinSourceError::Io {
            path: path.clone(),
            source,
        })?;
    if metadata.len() > MAX_SKIN_SOURCE_BYTES as u64 {
        return Err(SkinSourceError::SourceTooLarge);
    }
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|source| SkinSourceError::Io {
            path: path.clone(),
            source,
        })?;
    if bytes.len() > MAX_SKIN_SOURCE_BYTES {
        return Err(SkinSourceError::SourceTooLarge);
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("skin.png");
    prepare_skin(&bytes, file_name, None)
}

/// Downloads direct PNG links with explicit redirect and address checks.
#[derive(Clone)]
pub struct SkinUrlClient {
    client: reqwest::Client,
    allow_private: bool,
    allow_http: bool,
}

impl SkinUrlClient {
    pub fn new() -> Result<Self, SkinSourceError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("RMCL/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(SkinSourceError::Http)?;
        Ok(Self {
            client,
            allow_private: false,
            allow_http: false,
        })
    }

    /// Fetches a direct image link and prepares it for upload.
    pub async fn fetch(
        &self,
        url: &str,
        cancellation: &CancellationToken,
    ) -> Result<PreparedSkin, SkinSourceError> {
        let mut current = self.normalize_url(url)?;
        for redirect in 0..=MAX_REDIRECTS {
            if cancellation.is_cancelled() {
                return Err(SkinSourceError::Cancelled);
            }
            current = self.normalize_url(current.as_str())?;
            self.validate_host(&current).await?;
            let request = self.client.get(current.clone()).header(ACCEPT, "image/png");
            let mut response = tokio::select! {
                _ = cancellation.cancelled() => return Err(SkinSourceError::Cancelled),
                response = request.send() => response.map_err(SkinSourceError::Http)?,
            };
            let status = response.status();
            if status.is_redirection() {
                if redirect == MAX_REDIRECTS {
                    return Err(SkinSourceError::TooManyRedirects);
                }
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or(SkinSourceError::MissingLocation)?;
                current = current
                    .join(location)
                    .map_err(SkinSourceError::InvalidUrl)?;
                continue;
            }
            if !status.is_success() {
                return Err(SkinSourceError::HttpStatus { status });
            }
            if response
                .content_length()
                .is_some_and(|length| length > MAX_SKIN_SOURCE_BYTES as u64)
            {
                return Err(SkinSourceError::SourceTooLarge);
            }
            if let Some(content_type) = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                && !content_type.starts_with("image/png")
                && !content_type.starts_with("application/octet-stream")
            {
                return Err(SkinSourceError::NotPng);
            }

            let mut bytes = Vec::new();
            loop {
                let chunk = tokio::select! {
                    _ = cancellation.cancelled() => return Err(SkinSourceError::Cancelled),
                    chunk = response.chunk() => chunk.map_err(SkinSourceError::Http)?,
                };
                let Some(chunk) = chunk else {
                    break;
                };
                if bytes.len() + chunk.len() > MAX_SKIN_SOURCE_BYTES {
                    return Err(SkinSourceError::SourceTooLarge);
                }
                bytes.extend_from_slice(&chunk);
            }
            let file_name = current
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .filter(|name| !name.is_empty())
                .unwrap_or("skin.png");
            return prepare_skin(&bytes, file_name, Some(current.to_string()));
        }
        Err(SkinSourceError::TooManyRedirects)
    }

    fn normalize_url(&self, value: &str) -> Result<Url, SkinSourceError> {
        let mut url = Url::parse(value)?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err(SkinSourceError::CredentialsNotAllowed);
        }
        if url.scheme() == "http"
            && url.host_str() == Some("textures.minecraft.net")
            && !self.allow_http
        {
            let _ = url.set_scheme("https");
        }
        if url.scheme() != "https" && !(self.allow_http && url.scheme() == "http") {
            return Err(SkinSourceError::UnsupportedScheme);
        }
        url.set_fragment(None);
        Ok(url)
    }

    async fn validate_host(&self, url: &Url) -> Result<(), SkinSourceError> {
        let host = url.host_str().ok_or(SkinSourceError::MissingHost)?;
        let port = url.port_or_known_default().unwrap_or(443);
        if let Ok(address) = host.parse::<IpAddr>() {
            if !self.allow_private && is_blocked_address(address) {
                return Err(SkinSourceError::PrivateAddress);
            }
            return Ok(());
        }
        if self.allow_private {
            return Ok(());
        }
        let addresses = tokio::net::lookup_host((host, port))
            .await
            .map_err(SkinSourceError::Resolve)?
            .collect::<Vec<_>>();
        if addresses.is_empty() {
            return Err(SkinSourceError::Resolve(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "host resolved to no addresses",
            )));
        }
        if addresses
            .iter()
            .any(|address| is_blocked_address(address.ip()))
        {
            return Err(SkinSourceError::PrivateAddress);
        }
        Ok(())
    }

    #[cfg(test)]
    fn for_test() -> Self {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        Self {
            client,
            allow_private: true,
            allow_http: true,
        }
    }
}

fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, SkinSourceError> {
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(image.clone())
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .map_err(SkinSourceError::Encode)?;
    Ok(bytes)
}

/// Detects the model from the dead columns at the back of the right arm.
///
/// This is only a hint: the user's explicit model choice always wins.
fn detect_variant(image: &RgbaImage) -> SkinVariant {
    let scale = (image.width() / 64).max(1);
    let start_x = 54 * scale;
    let end_x = (56 * scale).min(image.width());
    let start_y = 20 * scale;
    let end_y = (32 * scale).min(image.height());
    let mut opaque = 0_u32;
    for y in start_y..end_y {
        for x in start_x..end_x {
            if image.get_pixel(x, y)[3] > 0 {
                opaque += 1;
            }
        }
    }
    let sample_count = (end_x - start_x).saturating_mul(end_y - start_y);
    if opaque < (sample_count / 4).max(1) {
        SkinVariant::Slim
    } else {
        SkinVariant::Classic
    }
}

fn normalized_file_name(value: &str) -> String {
    let name = Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("skin.png");
    if name.to_ascii_lowercase().ends_with(".png") {
        name.to_owned()
    } else {
        format!("{name}.png")
    }
}

pub(crate) fn is_blocked_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_blocked_ipv4(address),
        IpAddr::V6(address) => is_blocked_ipv6(address),
    }
}

fn is_blocked_ipv4(address: Ipv4Addr) -> bool {
    let octets = address.octets();
    address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_broadcast()
        || address.is_documentation()
        || address.is_multicast()
        || address.is_unspecified()
        || (octets[0] == 100 && (64..128).contains(&octets[1]))
}

fn is_blocked_ipv6(address: Ipv6Addr) -> bool {
    address.is_loopback()
        || address.is_unspecified()
        || address.is_multicast()
        || address.is_unique_local()
        || address.is_unicast_link_local()
}

#[derive(Debug, Error)]
pub enum SkinSourceError {
    #[error("invalid skin URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("skin URL is missing a host")]
    MissingHost,
    #[error("skin URL must use HTTPS")]
    UnsupportedScheme,
    #[error("skin URL must not contain credentials")]
    CredentialsNotAllowed,
    #[error("skin URL points to a private or local address")]
    PrivateAddress,
    #[error("failed to resolve skin URL host: {0}")]
    Resolve(#[source] std::io::Error),
    #[error("failed to download skin: {0}")]
    Http(#[source] reqwest::Error),
    #[error("skin download returned HTTP {status}")]
    HttpStatus { status: StatusCode },
    #[error("skin source exceeds the {} byte limit", MAX_SKIN_SOURCE_BYTES)]
    SourceTooLarge,
    #[error("skin URL does not point to a PNG image")]
    NotPng,
    #[error(transparent)]
    Skin(#[from] SkinError),
    #[error("official Java skins must be 64×64 or 64×32, got {width}×{height}")]
    UnsupportedUploadDimensions { width: u32, height: u32 },
    #[error(
        "skin file still exceeds the {} byte upload limit after normalization",
        MAX_SKIN_UPLOAD_BYTES
    )]
    UploadTooLarge,
    #[error("failed to encode skin PNG: {0}")]
    Encode(#[source] image::ImageError),
    #[error("failed to read skin file `{}`: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("skin download was cancelled")]
    Cancelled,
    #[error("skin URL redirected too many times")]
    TooManyRedirects,
    #[error("skin URL redirect has no Location header")]
    MissingLocation,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        Router,
        body::Body,
        extract::State,
        http::{StatusCode, header},
        response::{IntoResponse, Response},
        routing::get,
    };
    use image::{Rgba, RgbaImage};
    use tokio::net::TcpListener;

    use super::*;

    fn png(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        let image = RgbaImage::from_pixel(width, height, Rgba(color));
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(image)
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn prepares_modern_and_legacy_skins() {
        let modern = prepare_skin(&png(64, 64, [10, 20, 30, 255]), "my skin", None).unwrap();
        assert_eq!(modern.texture().dimensions(), (64, 64));
        assert_eq!(modern.file_name(), "my skin.png");
        assert_eq!(modern.variant_hint(), SkinVariant::Classic);

        let legacy = prepare_skin(&png(64, 32, [10, 20, 30, 255]), "legacy.png", None).unwrap();
        assert!(legacy.texture().is_legacy());
        assert_eq!(legacy.file_name(), "legacy.png");
    }

    #[test]
    fn detects_slim_from_the_dead_arm_columns() {
        let mut image = RgbaImage::from_pixel(64, 64, Rgba([0, 0, 0, 0]));
        for y in 0..64 {
            for x in 0..64 {
                if !(54..56).contains(&x) || !(20..32).contains(&y) {
                    image.put_pixel(x, y, Rgba([200, 100, 50, 255]));
                }
            }
        }
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(image)
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        let prepared = prepare_skin(&bytes, "slim.png", None).unwrap();

        assert_eq!(prepared.variant_hint(), SkinVariant::Slim);
    }

    #[test]
    fn rejects_hd_uploads() {
        let error = prepare_skin(&png(128, 128, [1, 2, 3, 255]), "hd.png", None).unwrap_err();
        assert!(matches!(
            error,
            SkinSourceError::UnsupportedUploadDimensions {
                width: 128,
                height: 128
            }
        ));
    }

    #[test]
    fn rejects_malformed_images() {
        let error = prepare_skin(b"not a png", "bad.png", None).unwrap_err();
        assert!(matches!(error, SkinSourceError::Skin(SkinError::Decode(_))));
    }

    #[tokio::test]
    async fn fetches_a_direct_png() {
        let body = png(64, 64, [200, 40, 40, 255]);
        let base = spawn_server(Serve::Body(body)).await;
        let client = SkinUrlClient::for_test();

        let prepared = client
            .fetch(&format!("{base}/skin.png"), &CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(prepared.file_name(), "skin.png");
        assert_eq!(prepared.texture().dimensions(), (64, 64));
        assert!(prepared.source_url().unwrap().starts_with(&base));
    }

    #[tokio::test]
    async fn follows_one_redirect() {
        let body = png(64, 64, [40, 200, 40, 255]);
        let base = spawn_server(Serve::Redirect(body)).await;
        let client = SkinUrlClient::for_test();

        let prepared = client
            .fetch(&format!("{base}/redirect"), &CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(prepared.texture().dimensions(), (64, 64));
    }

    #[tokio::test]
    async fn rejects_html_pages() {
        let base = spawn_server(Serve::Html).await;
        let client = SkinUrlClient::for_test();

        let error = client
            .fetch(&format!("{base}/page"), &CancellationToken::new())
            .await
            .unwrap_err();

        assert!(matches!(error, SkinSourceError::NotPng));
    }

    #[tokio::test]
    async fn rejects_private_addresses_in_production_mode() {
        let client = SkinUrlClient::new().unwrap();

        let error = client
            .fetch("https://127.0.0.1/skin.png", &CancellationToken::new())
            .await
            .unwrap_err();

        assert!(matches!(error, SkinSourceError::PrivateAddress));
    }

    #[tokio::test]
    async fn rejects_oversized_bodies() {
        let base = spawn_server(Serve::TooLarge).await;
        let client = SkinUrlClient::for_test();

        let error = client
            .fetch(&format!("{base}/large.png"), &CancellationToken::new())
            .await
            .unwrap_err();

        assert!(matches!(error, SkinSourceError::SourceTooLarge));
    }

    #[derive(Clone)]
    enum Serve {
        Body(Vec<u8>),
        Redirect(Vec<u8>),
        Html,
        TooLarge,
    }

    #[derive(Clone)]
    struct ServerState {
        serve: Arc<Serve>,
    }

    async fn spawn_server(serve: Serve) -> String {
        let state = ServerState {
            serve: Arc::new(serve),
        };
        let router = Router::new()
            .route("/skin.png", get(serve_skin))
            .route("/redirect", get(serve_redirect))
            .route("/page", get(serve_page))
            .route("/large.png", get(serve_large))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        format!("http://{address}")
    }

    async fn serve_skin(State(state): State<ServerState>) -> Response {
        match state.serve.as_ref() {
            Serve::Body(body) | Serve::Redirect(body) => {
                ([(header::CONTENT_TYPE, "image/png")], body.clone()).into_response()
            }
            _ => StatusCode::NOT_FOUND.into_response(),
        }
    }

    async fn serve_redirect(State(state): State<ServerState>) -> Response {
        match state.serve.as_ref() {
            Serve::Redirect(_) => {
                (StatusCode::FOUND, [(header::LOCATION, "/skin.png")]).into_response()
            }
            _ => StatusCode::NOT_FOUND.into_response(),
        }
    }

    async fn serve_page() -> Response {
        (
            [(header::CONTENT_TYPE, "text/html")],
            Body::from("<html>not a skin</html>"),
        )
            .into_response()
    }

    async fn serve_large() -> Response {
        (
            [(header::CONTENT_TYPE, "image/png")],
            Body::from(vec![0_u8; MAX_SKIN_SOURCE_BYTES + 1]),
        )
            .into_response()
    }
}
