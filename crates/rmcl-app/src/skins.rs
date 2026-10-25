//! Official Minecraft skin texture fetching and head rendering.

use std::{
    io::{Cursor, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use image::{ImageFormat, Rgba, RgbaImage, imageops::FilterType};
use reqwest::StatusCode;
use sha1::{Digest, Sha1};
use thiserror::Error;
use url::Url;

const OFFICIAL_SKIN_HOST: &str = "textures.minecraft.net";
const MAX_SKIN_BYTES: usize = 1024 * 1024;
const HEAD_SIZE: u32 = 64;

/// Downloads official skin textures and caches a composed head image.
#[derive(Clone)]
pub struct SkinCache {
    client: reqwest::Client,
    cache_dir: PathBuf,
    allowed_host: String,
}

impl SkinCache {
    /// Creates a cache that only accepts official Minecraft texture URLs.
    pub fn new(cache_dir: impl Into<PathBuf>) -> Self {
        Self {
            client: reqwest::Client::new(),
            cache_dir: cache_dir.into(),
            allowed_host: OFFICIAL_SKIN_HOST.to_owned(),
        }
    }

    /// Returns a cached 64×64 head PNG, downloading and composing it if needed.
    pub async fn ensure_head(&self, skin_url: &str) -> Result<PathBuf, SkinError> {
        let url = self.validate_url(skin_url)?;
        let cache_path = self
            .cache_dir
            .join(format!("{}.png", hash_url(url.as_str())));
        if tokio::fs::metadata(&cache_path)
            .await
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)
        {
            return Ok(cache_path);
        }

        let response = self
            .client
            .get(url.clone())
            .header("User-Agent", concat!("RMCL/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|source| SkinError::Http { source })?;
        let status = response.status();
        if response.url().host_str() != Some(self.allowed_host.as_str()) {
            return Err(SkinError::HostNotAllowed);
        }
        if !status.is_success() {
            return Err(SkinError::HttpStatus { status });
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_SKIN_BYTES as u64)
        {
            return Err(SkinError::TooLarge);
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|source| SkinError::Http { source })?;
        if bytes.len() > MAX_SKIN_BYTES {
            return Err(SkinError::TooLarge);
        }

        let cache_path_for_write = cache_path.clone();
        tokio::task::spawn_blocking(move || {
            let head = compose_head(&bytes)?;
            write_cache(&cache_path_for_write, &head)?;
            Ok::<_, SkinError>(())
        })
        .await
        .map_err(|error| SkinError::Blocking(error.to_string()))??;
        Ok(cache_path)
    }

    fn validate_url(&self, skin_url: &str) -> Result<Url, SkinError> {
        let url = Url::parse(skin_url).map_err(SkinError::InvalidUrl)?;
        if url.scheme() != "https" && url.scheme() != "http" {
            return Err(SkinError::UnsupportedScheme);
        }
        if url.host_str() != Some(self.allowed_host.as_str()) {
            return Err(SkinError::HostNotAllowed);
        }
        Ok(url)
    }

    #[cfg(test)]
    fn with_host(cache_dir: impl Into<PathBuf>, host: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            cache_dir: cache_dir.into(),
            allowed_host: host.to_owned(),
        }
    }
}

/// Builds a 64×64 head image from a 64×32 or larger skin texture.
fn compose_head(skin_bytes: &[u8]) -> Result<Vec<u8>, SkinError> {
    let skin = image::load_from_memory_with_format(skin_bytes, ImageFormat::Png)
        .map_err(SkinError::Decode)?
        .to_rgba8();
    let (width, height) = skin.dimensions();
    if width < 64 || height < 32 || width % 64 != 0 {
        return Err(SkinError::InvalidDimensions { width, height });
    }
    let scale = width / 64;
    let face_size = 8 * scale;
    let mut head = RgbaImage::new(face_size, face_size);
    for y in 0..face_size {
        for x in 0..face_size {
            let face = *skin.get_pixel(8 * scale + x, 8 * scale + y);
            let pixel = if height >= 16 * scale {
                let hat = *skin.get_pixel(40 * scale + x, 8 * scale + y);
                blend(face, hat)
            } else {
                face
            };
            head.put_pixel(x, y, pixel);
        }
    }
    let head = image::imageops::resize(&head, HEAD_SIZE, HEAD_SIZE, FilterType::Nearest);
    let mut encoded = Vec::new();
    image::DynamicImage::ImageRgba8(head)
        .write_to(&mut Cursor::new(&mut encoded), ImageFormat::Png)
        .map_err(SkinError::Encode)?;
    Ok(encoded)
}

fn blend(base: Rgba<u8>, overlay: Rgba<u8>) -> Rgba<u8> {
    let alpha = overlay[3] as f32 / 255.0;
    if alpha <= 0.0 {
        return base;
    }
    if alpha >= 1.0 {
        return Rgba([overlay[0], overlay[1], overlay[2], 255]);
    }
    let base_alpha = base[3] as f32 / 255.0;
    let output_alpha = alpha + base_alpha * (1.0 - alpha);
    if output_alpha <= 0.0 {
        return Rgba([0, 0, 0, 0]);
    }
    let channel = |index: usize| {
        (((overlay[index] as f32 * alpha) + (base[index] as f32 * base_alpha * (1.0 - alpha)))
            / output_alpha)
            .round() as u8
    };
    Rgba([
        channel(0),
        channel(1),
        channel(2),
        (output_alpha * 255.0).round() as u8,
    ])
}

fn write_cache(path: &Path, bytes: &[u8]) -> Result<(), SkinError> {
    let parent = path.parent().ok_or_else(|| SkinError::Io {
        operation: "find skin cache directory",
        path: path.to_path_buf(),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "skin cache path has no parent",
        ),
    })?;
    std::fs::create_dir_all(parent).map_err(|source| SkinError::Io {
        operation: "create skin cache directory",
        path: parent.to_path_buf(),
        source,
    })?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|source| SkinError::Io {
            operation: "create skin cache temporary file",
            path: parent.to_path_buf(),
            source,
        })?;
    temporary.write_all(bytes).map_err(|source| SkinError::Io {
        operation: "write skin cache",
        path: temporary.path().to_path_buf(),
        source,
    })?;
    temporary.persist(path).map_err(|error| SkinError::Io {
        operation: "publish skin cache",
        path: path.to_path_buf(),
        source: error.error,
    })?;
    Ok(())
}

fn hash_url(url: &str) -> String {
    let mut digest = Sha1::new();
    digest.update(url.as_bytes());
    hex::encode(digest.finalize())
}

#[derive(Debug, Error)]
pub enum SkinError {
    #[error("invalid skin URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("skin URL must use HTTP or HTTPS")]
    UnsupportedScheme,
    #[error("skin URL does not point to an allowed Minecraft texture host")]
    HostNotAllowed,
    #[error("failed to download skin texture: {source}")]
    Http {
        #[source]
        source: reqwest::Error,
    },
    #[error("skin texture download returned HTTP {status}")]
    HttpStatus { status: StatusCode },
    #[error("skin texture exceeds the {} byte limit", MAX_SKIN_BYTES)]
    TooLarge,
    #[error("skin texture is not a valid PNG: {0}")]
    Decode(#[source] image::ImageError),
    #[error("skin texture is {width}×{height}; expected at least 64×32 and a multiple of 64 wide")]
    InvalidDimensions { width: u32, height: u32 },
    #[error("failed to encode skin head: {0}")]
    Encode(#[source] image::ImageError),
    #[error("failed to {operation} `{path}`: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("skin processing task failed: {0}")]
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
        response::{IntoResponse, Response},
        routing::get,
    };
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn downloads_and_caches_a_composed_head() {
        let (base, requests) =
            spawn_skin_server(skin_png([220, 40, 40, 255], Some([20, 80, 220, 128]))).await;
        let temporary = TempDir::new().unwrap();
        let cache = SkinCache::with_host(temporary.path().to_path_buf(), "127.0.0.1");

        let path = cache
            .ensure_head(&format!("{base}/texture/abc"))
            .await
            .unwrap();

        assert!(path.exists());
        let head = image::load_from_memory(&std::fs::read(&path).unwrap())
            .unwrap()
            .to_rgba8();
        assert_eq!(head.dimensions(), (64, 64));
        let pixel = head.get_pixel(0, 0);
        assert!(pixel[0] > 100 && pixel[2] > 100);
        assert_eq!(requests.load(Ordering::SeqCst), 1);

        let again = cache
            .ensure_head(&format!("{base}/texture/abc"))
            .await
            .unwrap();
        assert_eq!(again, path);
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn rejects_non_official_hosts_before_network_access() {
        let temporary = TempDir::new().unwrap();
        let cache = SkinCache::new(temporary.path().to_path_buf());

        let error = cache
            .ensure_head("https://example.com/texture/abc")
            .await
            .unwrap_err();

        assert!(matches!(error, SkinError::HostNotAllowed));
    }

    #[tokio::test]
    async fn rejects_invalid_skin_dimensions() {
        let (base, _) = spawn_skin_server(solid_png(32, 32, [255, 0, 0, 255])).await;
        let temporary = TempDir::new().unwrap();
        let cache = SkinCache::with_host(temporary.path().to_path_buf(), "127.0.0.1");

        let error = cache
            .ensure_head(&format!("{base}/texture/abc"))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            SkinError::InvalidDimensions {
                width: 32,
                height: 32
            }
        ));
    }

    #[tokio::test]
    #[ignore = "downloads from Mojang texture servers"]
    async fn official_skin_downloads_and_composes() {
        let temporary = TempDir::new().unwrap();
        let cache = SkinCache::new(temporary.path().to_path_buf());
        let path = cache
            .ensure_head(
                "https://textures.minecraft.net/texture/1a4af718455d4aab528e7a61f86fa25e6a369d1768dcb13f7df319a713eb810b",
            )
            .await
            .unwrap();
        let head = image::load_from_memory(&std::fs::read(path).unwrap())
            .unwrap()
            .to_rgba8();
        assert_eq!(head.dimensions(), (64, 64));
    }

    async fn spawn_skin_server(body: Vec<u8>) -> (String, Arc<AtomicUsize>) {
        #[derive(Clone)]
        struct SkinState {
            body: Arc<Vec<u8>>,
            requests: Arc<AtomicUsize>,
        }

        async fn texture(State(state): State<SkinState>) -> Response {
            state.requests.fetch_add(1, Ordering::SeqCst);
            ([("content-type", "image/png")], state.body.as_ref().clone()).into_response()
        }

        let state = SkinState {
            body: Arc::new(body),
            requests: Arc::new(AtomicUsize::new(0)),
        };
        let requests = state.requests.clone();
        let router = Router::new()
            .route("/texture/abc", get(texture))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (format!("http://{address}"), requests)
    }

    fn skin_png(face: [u8; 4], hat: Option<[u8; 4]>) -> Vec<u8> {
        let mut image = RgbaImage::from_pixel(64, 64, Rgba([0, 0, 0, 0]));
        for y in 0..8 {
            for x in 0..8 {
                image.put_pixel(8 + x, 8 + y, Rgba(face));
            }
        }
        if let Some(hat) = hat {
            for y in 0..8 {
                for x in 0..8 {
                    image.put_pixel(40 + x, 8 + y, Rgba(hat));
                }
            }
        }
        encode_png(image)
    }

    fn solid_png(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        encode_png(RgbaImage::from_pixel(width, height, Rgba(color)))
    }

    fn encode_png(image: RgbaImage) -> Vec<u8> {
        let mut encoded = Vec::new();
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut Cursor::new(&mut encoded), ImageFormat::Png)
            .unwrap();
        encoded
    }
}
