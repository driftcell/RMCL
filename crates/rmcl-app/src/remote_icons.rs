//! Downloads and caches remote project icons for the content browser.
//!
//! Icons come from provider metadata as HTTPS URLs. The UI needs local paths so
//! it can reuse GPUI's file image loader and keep working offline after the
//! first request.

use std::{
    io::{Cursor, Write},
    net::IpAddr,
    path::{Path, PathBuf},
    time::Duration,
};

use image::{DynamicImage, GenericImageView, ImageFormat};
use reqwest::{
    StatusCode,
    header::{ACCEPT, CONTENT_TYPE},
};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::skin_source::is_blocked_address;

/// Maximum bytes accepted from one remote icon before decoding.
pub const MAX_REMOTE_ICON_BYTES: usize = 2 * 1024 * 1024;
const MAX_ICON_DIMENSION: u32 = 2048;
const CACHED_ICON_DIMENSION: u32 = 256;

/// A content browser icon downloaded by RMCL and stored as a normalized PNG.
#[derive(Debug, Clone)]
pub struct RemoteIconCache {
    directory: PathBuf,
    client: reqwest::Client,
    allow_private: bool,
    allow_http: bool,
}

impl RemoteIconCache {
    pub fn new(directory: impl Into<PathBuf>) -> Result<Self, RemoteIconError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("RMCL/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(RemoteIconError::Http)?;
        Ok(Self {
            directory: directory.into(),
            client,
            allow_private: false,
            allow_http: false,
        })
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Returns a local PNG path, downloading and validating the source once.
    pub async fn ensure(
        &self,
        source: &str,
        cancellation: &CancellationToken,
    ) -> Result<PathBuf, RemoteIconError> {
        let url = normalize_icon_url(source, self.allow_http)?;
        let path = self.path_for_url(url.as_str());
        if tokio::fs::metadata(&path)
            .await
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)
        {
            return Ok(path);
        }

        self.validate_host(&url).await?;
        let request = self.client.get(url.clone()).header(
            ACCEPT,
            "image/avif,image/webp,image/png,image/jpeg,image/*;q=0.8",
        );
        let mut response = tokio::select! {
            _ = cancellation.cancelled() => return Err(RemoteIconError::Cancelled),
            response = request.send() => response.map_err(RemoteIconError::Http)?,
        };
        if response.status().is_redirection() {
            return Err(RemoteIconError::Redirect);
        }
        if !response.status().is_success() {
            return Err(RemoteIconError::HttpStatus {
                status: response.status(),
            });
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_REMOTE_ICON_BYTES as u64)
        {
            return Err(RemoteIconError::SourceTooLarge);
        }
        if let Some(content_type) = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            && !content_type.starts_with("image/")
            && !content_type.starts_with("application/octet-stream")
        {
            return Err(RemoteIconError::NotAnImage);
        }

        let mut bytes = Vec::new();
        loop {
            let chunk = tokio::select! {
                _ = cancellation.cancelled() => return Err(RemoteIconError::Cancelled),
                chunk = response.chunk() => chunk.map_err(RemoteIconError::Http)?,
            };
            let Some(chunk) = chunk else {
                break;
            };
            if bytes.len() + chunk.len() > MAX_REMOTE_ICON_BYTES {
                return Err(RemoteIconError::SourceTooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }

        let png = tokio::task::spawn_blocking(move || prepare_icon(&bytes))
            .await
            .map_err(|error| RemoteIconError::Blocking(error.to_string()))??;
        self.write(&path, &png).await?;
        Ok(path)
    }

    pub fn path_for_url(&self, url: &str) -> PathBuf {
        let mut digest = Sha256::new();
        digest.update(url.as_bytes());
        self.directory
            .join(format!("{}.png", hex::encode(digest.finalize())))
    }

    async fn validate_host(&self, url: &Url) -> Result<(), RemoteIconError> {
        let host = url.host_str().ok_or(RemoteIconError::MissingHost)?;
        let port = url.port_or_known_default().unwrap_or(443);
        if let Ok(address) = host.parse::<IpAddr>() {
            if !self.allow_private && is_blocked_address(address) {
                return Err(RemoteIconError::PrivateAddress);
            }
            return Ok(());
        }
        if self.allow_private {
            return Ok(());
        }
        let addresses = tokio::net::lookup_host((host, port))
            .await
            .map_err(RemoteIconError::Resolve)?
            .collect::<Vec<_>>();
        if addresses.is_empty() {
            return Err(RemoteIconError::Resolve(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "host resolved to no addresses",
            )));
        }
        if addresses
            .iter()
            .any(|address| is_blocked_address(address.ip()))
        {
            return Err(RemoteIconError::PrivateAddress);
        }
        Ok(())
    }

    async fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), RemoteIconError> {
        let directory = self.directory.clone();
        let path = path.to_path_buf();
        let bytes = bytes.to_vec();
        tokio::task::spawn_blocking(move || write_icon(&directory, &path, &bytes))
            .await
            .map_err(|error| RemoteIconError::Blocking(error.to_string()))?
    }

    #[cfg(test)]
    fn for_test(directory: impl Into<PathBuf>) -> Self {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        Self {
            directory: directory.into(),
            client,
            allow_private: true,
            allow_http: true,
        }
    }
}

fn normalize_icon_url(value: &str, allow_http: bool) -> Result<Url, RemoteIconError> {
    let mut url = Url::parse(value)?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(RemoteIconError::CredentialsNotAllowed);
    }
    if url.scheme() != "https" && !(allow_http && url.scheme() == "http") {
        return Err(RemoteIconError::UnsupportedScheme);
    }
    url.set_fragment(None);
    Ok(url)
}

fn prepare_icon(bytes: &[u8]) -> Result<Vec<u8>, RemoteIconError> {
    let image = image::load_from_memory(bytes).map_err(RemoteIconError::Decode)?;
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 || width > MAX_ICON_DIMENSION || height > MAX_ICON_DIMENSION {
        return Err(RemoteIconError::InvalidDimensions { width, height });
    }
    let image = if width > CACHED_ICON_DIMENSION || height > CACHED_ICON_DIMENSION {
        image.thumbnail(CACHED_ICON_DIMENSION, CACHED_ICON_DIMENSION)
    } else {
        image
    };
    let mut png = Vec::new();
    DynamicImage::ImageRgba8(image.to_rgba8())
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .map_err(RemoteIconError::Encode)?;
    Ok(png)
}

fn write_icon(directory: &Path, path: &Path, bytes: &[u8]) -> Result<(), RemoteIconError> {
    std::fs::create_dir_all(directory).map_err(|source| RemoteIconError::Cache {
        path: directory.to_path_buf(),
        source,
    })?;
    let mut temporary =
        NamedTempFile::new_in(directory).map_err(|source| RemoteIconError::Cache {
            path: directory.to_path_buf(),
            source,
        })?;
    temporary
        .write_all(bytes)
        .map_err(|source| RemoteIconError::Cache {
            path: temporary.path().to_path_buf(),
            source,
        })?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|source| RemoteIconError::Cache {
            path: temporary.path().to_path_buf(),
            source,
        })?;
    match temporary.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(RemoteIconError::Cache {
            path: path.to_path_buf(),
            source: error.error,
        }),
    }
}

#[derive(Debug, Error)]
pub enum RemoteIconError {
    #[error("invalid icon URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("icon URL is missing a host")]
    MissingHost,
    #[error("icon URL must use HTTPS")]
    UnsupportedScheme,
    #[error("icon URL must not contain credentials")]
    CredentialsNotAllowed,
    #[error("icon URL points to a private or local address")]
    PrivateAddress,
    #[error("failed to resolve icon URL host: {0}")]
    Resolve(#[source] std::io::Error),
    #[error("failed to download icon: {0}")]
    Http(#[source] reqwest::Error),
    #[error("icon download returned HTTP {status}")]
    HttpStatus { status: StatusCode },
    #[error("icon download was cancelled")]
    Cancelled,
    #[error("icon URL redirected; redirects are not followed")]
    Redirect,
    #[error("icon source exceeds the {} byte limit", MAX_REMOTE_ICON_BYTES)]
    SourceTooLarge,
    #[error("icon source is not an image")]
    NotAnImage,
    #[error("icon dimensions {width}×{height} are invalid")]
    InvalidDimensions { width: u32, height: u32 },
    #[error("failed to decode icon: {0}")]
    Decode(#[source] image::ImageError),
    #[error("failed to encode icon PNG: {0}")]
    Encode(#[source] image::ImageError),
    #[error("failed to cache icon at `{}`: {source}", path.display())]
    Cache {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("icon processing task failed: {0}")]
    Blocking(String),
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use axum::{
        Router,
        extract::State,
        http::{StatusCode, header},
        response::IntoResponse,
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
    fn normalizes_urls_and_rejects_unsafe_sources() {
        assert_eq!(
            normalize_icon_url("https://cdn.example.com/icon.webp#fragment", false)
                .unwrap()
                .as_str(),
            "https://cdn.example.com/icon.webp"
        );
        assert!(matches!(
            normalize_icon_url("http://cdn.example.com/icon.png", false),
            Err(RemoteIconError::UnsupportedScheme)
        ));
        assert!(matches!(
            normalize_icon_url("https://user:pass@cdn.example.com/icon.png", false),
            Err(RemoteIconError::CredentialsNotAllowed)
        ));
    }

    #[test]
    fn prepares_and_downscales_icons_to_png() {
        let bytes = png(512, 512, [10, 20, 30, 255]);
        let prepared = prepare_icon(&bytes).unwrap();
        let image = image::load_from_memory(&prepared).unwrap();
        assert_eq!(image.dimensions(), (256, 256));

        let tiny = png(1, 1, [1, 2, 3, 255]);
        assert!(
            prepare_icon(&tiny)
                .unwrap()
                .starts_with(b"\x89PNG\r\n\x1a\n")
        );
        assert!(matches!(
            prepare_icon(b"not an image"),
            Err(RemoteIconError::Decode(_))
        ));
    }

    #[tokio::test]
    async fn downloads_once_and_reuses_the_cached_png() {
        let requests = Arc::new(AtomicUsize::new(0));
        let state = requests.clone();
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new()
            .route(
                "/icon.png",
                get(|State(requests): State<Arc<AtomicUsize>>| async move {
                    requests.fetch_add(1, Ordering::SeqCst);
                    (
                        StatusCode::OK,
                        [(header::CONTENT_TYPE, "image/png")],
                        png(64, 64, [4, 5, 6, 255]),
                    )
                        .into_response()
                }),
            )
            .with_state(state);
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let temporary = tempfile::TempDir::new().unwrap();
        let cache = RemoteIconCache::for_test(temporary.path());
        let url = format!("http://{address}/icon.png");
        let cancellation = CancellationToken::new();
        let first = cache.ensure(&url, &cancellation).await.unwrap();
        let second = cache.ensure(&url, &cancellation).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        assert_eq!(
            image::load_from_memory(&std::fs::read(&first).unwrap())
                .unwrap()
                .dimensions(),
            (64, 64)
        );
        server.abort();
    }

    #[tokio::test]
    async fn rejects_private_icon_hosts_before_downloading() {
        let temporary = tempfile::TempDir::new().unwrap();
        let cache = RemoteIconCache::new(temporary.path()).unwrap();
        let error = cache
            .ensure("https://127.0.0.1/icon.png", &CancellationToken::new())
            .await
            .unwrap_err();
        assert!(matches!(error, RemoteIconError::PrivateAddress));
    }
}
