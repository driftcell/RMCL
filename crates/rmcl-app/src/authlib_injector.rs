//! Download and cache the authlib-injector Java agent.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use reqwest::{Client, StatusCode};
use rmcl_net::MirrorHealth;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{fs, io::AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

const DEFAULT_MANIFEST_URL: &str = "https://authlib-injector.yushi.moe/artifact/latest.json";
const MANIFEST_MAX_BYTES: usize = 64 * 1024;
const ARTIFACT_MAX_BYTES: u64 = 16 * 1024 * 1024;
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const MANIFEST_TIMEOUT: Duration = Duration::from_secs(20);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const USER_AGENT: &str = concat!("RMCL/", env!("CARGO_PKG_VERSION"));

/// A verified authlib-injector jar ready to pass to the JVM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthlibInjectorArtifact {
    pub path: PathBuf,
    pub version: String,
    pub build_number: u64,
}

/// Downloads, verifies, and caches authlib-injector artifacts.
#[derive(Clone)]
pub struct AuthlibInjectorService {
    client: Client,
    mirror: Arc<MirrorHealth>,
    cache_dir: PathBuf,
    manifest_url: Url,
    last_check: Arc<Mutex<Option<Instant>>>,
}

impl std::fmt::Debug for AuthlibInjectorService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthlibInjectorService")
            .field("cache_dir", &self.cache_dir)
            .field("manifest_url", &self.manifest_url)
            .finish_non_exhaustive()
    }
}

impl AuthlibInjectorService {
    pub fn new(
        mirror: Arc<MirrorHealth>,
        cache_dir: impl Into<PathBuf>,
    ) -> Result<Self, AuthlibInjectorError> {
        let manifest_url = Url::parse(DEFAULT_MANIFEST_URL)
            .map_err(|error| AuthlibInjectorError::InvalidDownloadUrl(error.to_string()))?;
        Self::with_endpoints(mirror, cache_dir, manifest_url)
    }

    /// Builds a service with an explicit manifest endpoint for tests.
    pub fn with_endpoints(
        mirror: Arc<MirrorHealth>,
        cache_dir: impl Into<PathBuf>,
        manifest_url: Url,
    ) -> Result<Self, AuthlibInjectorError> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(AuthlibInjectorError::Client)?;
        Ok(Self {
            client,
            mirror,
            cache_dir: cache_dir.into(),
            manifest_url,
            last_check: Arc::new(Mutex::new(None)),
        })
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Returns a verified injector jar, downloading it when necessary.
    ///
    /// The latest manifest is checked at most once per process day. When the
    /// network is unavailable, the newest valid cached artifact is used.
    pub async fn ensure(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<AuthlibInjectorArtifact, AuthlibInjectorError> {
        let cached = self.cached_artifact().await?;
        if !self.should_check() {
            return cached.ok_or(AuthlibInjectorError::NotCached);
        }

        match self.fetch_manifest(cancellation).await {
            Ok(manifest) => match self.ensure_manifest(&manifest, cancellation).await {
                Ok(artifact) => {
                    self.mark_checked();
                    Ok(artifact)
                }
                Err(error) => {
                    if let Some(cached) = cached {
                        self.mark_checked();
                        Ok(cached)
                    } else {
                        Err(error)
                    }
                }
            },
            Err(error) => {
                if let Some(cached) = cached {
                    self.mark_checked();
                    Ok(cached)
                } else {
                    Err(error)
                }
            }
        }
    }

    fn should_check(&self) -> bool {
        let last_check = self
            .last_check
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        last_check.is_none_or(|checked| checked.elapsed() >= CHECK_INTERVAL)
    }

    fn mark_checked(&self) {
        let mut last_check = self
            .last_check
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *last_check = Some(Instant::now());
    }

    async fn fetch_manifest(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<AuthlibInjectorManifest, AuthlibInjectorError> {
        let sources = self.mirror.sources_for(&self.manifest_url);
        let mut failures = Vec::new();
        for source in sources {
            match self.fetch_manifest_from(&source, cancellation).await {
                Ok(manifest) => return Ok(manifest),
                Err(AuthlibInjectorError::Cancelled) => {
                    return Err(AuthlibInjectorError::Cancelled);
                }
                Err(error) => failures.push(format!("{source}: {error}")),
            }
        }
        Err(AuthlibInjectorError::AllSourcesFailed {
            attempts: failures.join("; "),
        })
    }

    async fn fetch_manifest_from(
        &self,
        source: &Url,
        cancellation: &CancellationToken,
    ) -> Result<AuthlibInjectorManifest, AuthlibInjectorError> {
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(AuthlibInjectorError::Cancelled),
            result = self.client.get(source.clone()).timeout(MANIFEST_TIMEOUT).send() => {
                result.map_err(AuthlibInjectorError::Http)?
            }
        };
        let status = response.status();
        if !status.is_success() {
            return Err(AuthlibInjectorError::HttpStatus {
                url: source.to_string(),
                status,
            });
        }
        let body = read_body_limited(response, MANIFEST_MAX_BYTES, cancellation).await?;
        let raw: RawArtifact = serde_json::from_slice(&body).map_err(AuthlibInjectorError::Json)?;
        let sha256 = raw
            .checksums
            .get("sha256")
            .filter(|value| is_sha256(value))
            .ok_or(AuthlibInjectorError::MissingChecksum)?
            .to_ascii_lowercase();
        if raw.version.trim().is_empty() {
            return Err(AuthlibInjectorError::InvalidManifest);
        }
        Ok(AuthlibInjectorManifest {
            build_number: raw.build_number,
            version: raw.version,
            download_url: raw.download_url,
            sha256,
        })
    }

    async fn ensure_manifest(
        &self,
        manifest: &AuthlibInjectorManifest,
        cancellation: &CancellationToken,
    ) -> Result<AuthlibInjectorArtifact, AuthlibInjectorError> {
        fs::create_dir_all(&self.cache_dir)
            .await
            .map_err(|source| AuthlibInjectorError::Io {
                path: self.cache_dir.display().to_string(),
                source,
            })?;
        let artifact_path = self.artifact_path(manifest.build_number);
        if artifact_path.is_file() && self.verify_file(&artifact_path, &manifest.sha256).await? {
            let _ = self.write_cached_manifest(manifest).await;
            return Ok(manifest.artifact(artifact_path));
        }

        let download_url = Url::parse(&manifest.download_url)
            .map_err(|error| AuthlibInjectorError::InvalidDownloadUrl(error.to_string()))?;
        if !matches!(download_url.scheme(), "http" | "https") {
            return Err(AuthlibInjectorError::InvalidDownloadUrl(
                "download URL must use HTTP or HTTPS".to_owned(),
            ));
        }
        let sources = self.mirror.sources_for(&download_url);
        let mut failures = Vec::new();
        for source in sources {
            if !matches!(source.scheme(), "http" | "https") {
                failures.push(format!("{source}: unsupported scheme"));
                continue;
            }
            match self
                .download_artifact(&source, &manifest.sha256, cancellation)
                .await
            {
                Ok((temporary, actual)) => {
                    if actual != manifest.sha256 {
                        let _ = fs::remove_file(&temporary).await;
                        failures.push(format!(
                            "{source}: checksum mismatch (expected {}, got {actual})",
                            manifest.sha256
                        ));
                        continue;
                    }
                    if artifact_path.exists() {
                        fs::remove_file(&artifact_path)
                            .await
                            .map_err(|source| AuthlibInjectorError::Io {
                                path: artifact_path.display().to_string(),
                                source,
                            })?;
                    }
                    fs::rename(&temporary, &artifact_path)
                        .await
                        .map_err(|source| AuthlibInjectorError::Io {
                            path: artifact_path.display().to_string(),
                            source,
                        })?;
                    let _ = self.write_cached_manifest(manifest).await;
                    return Ok(manifest.artifact(artifact_path));
                }
                Err(AuthlibInjectorError::Cancelled) => {
                    return Err(AuthlibInjectorError::Cancelled);
                }
                Err(error) => failures.push(format!("{source}: {error}")),
            }
        }
        Err(AuthlibInjectorError::AllSourcesFailed {
            attempts: failures.join("; "),
        })
    }

    async fn download_artifact(
        &self,
        source: &Url,
        expected_sha256: &str,
        cancellation: &CancellationToken,
    ) -> Result<(PathBuf, String), AuthlibInjectorError> {
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(AuthlibInjectorError::Cancelled),
            result = self.client.get(source.clone()).send() => {
                result.map_err(AuthlibInjectorError::Http)?
            }
        };
        let status = response.status();
        if !status.is_success() {
            return Err(AuthlibInjectorError::HttpStatus {
                url: source.to_string(),
                status,
            });
        }
        if let Some(length) = response.content_length()
            && length > ARTIFACT_MAX_BYTES
        {
            return Err(AuthlibInjectorError::ArtifactTooLarge {
                limit: ARTIFACT_MAX_BYTES,
            });
        }

        let temporary = self
            .cache_dir
            .join(format!(".authlib-injector-{}.tmp", Uuid::new_v4().simple()));
        let mut file =
            fs::File::create(&temporary)
                .await
                .map_err(|source| AuthlibInjectorError::Io {
                    path: temporary.display().to_string(),
                    source,
                })?;
        let result = self
            .write_response(response, &mut file, &temporary, cancellation)
            .await;
        let actual = match result {
            Ok(actual) => actual,
            Err(error) => {
                let _ = fs::remove_file(&temporary).await;
                return Err(error);
            }
        };
        file.sync_all()
            .await
            .map_err(|source| AuthlibInjectorError::Io {
                path: temporary.display().to_string(),
                source,
            })?;
        drop(file);
        if actual != expected_sha256 {
            let _ = fs::remove_file(&temporary).await;
            return Err(AuthlibInjectorError::ChecksumMismatch {
                expected: expected_sha256.to_owned(),
                actual,
            });
        }
        Ok((temporary, actual))
    }

    async fn write_response(
        &self,
        mut response: reqwest::Response,
        file: &mut fs::File,
        temporary: &Path,
        cancellation: &CancellationToken,
    ) -> Result<String, AuthlibInjectorError> {
        let mut hasher = Sha256::new();
        let mut total = 0_u64;
        loop {
            let chunk = tokio::select! {
                _ = cancellation.cancelled() => return Err(AuthlibInjectorError::Cancelled),
                chunk = response.chunk() => chunk.map_err(AuthlibInjectorError::Http)?,
            };
            let Some(chunk) = chunk else {
                break;
            };
            total = total.saturating_add(chunk.len() as u64);
            if total > ARTIFACT_MAX_BYTES {
                return Err(AuthlibInjectorError::ArtifactTooLarge {
                    limit: ARTIFACT_MAX_BYTES,
                });
            }
            hasher.update(&chunk);
            file.write_all(&chunk)
                .await
                .map_err(|source| AuthlibInjectorError::Io {
                    path: temporary.display().to_string(),
                    source,
                })?;
        }
        Ok(hex::encode(hasher.finalize()))
    }

    async fn cached_artifact(
        &self,
    ) -> Result<Option<AuthlibInjectorArtifact>, AuthlibInjectorError> {
        let mut entries = match fs::read_dir(&self.cache_dir).await {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(AuthlibInjectorError::Io {
                    path: self.cache_dir.display().to_string(),
                    source,
                });
            }
        };
        let mut best: Option<AuthlibInjectorArtifact> = None;
        while let Some(entry) =
            entries
                .next_entry()
                .await
                .map_err(|source| AuthlibInjectorError::Io {
                    path: self.cache_dir.display().to_string(),
                    source,
                })?
        {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if !name.starts_with("authlib-injector-") {
                continue;
            }
            let Ok(bytes) = fs::read(&path).await else {
                continue;
            };
            let Ok(cached) = serde_json::from_slice::<CachedArtifact>(&bytes) else {
                continue;
            };
            let artifact_path = self.artifact_path(cached.build_number);
            if !artifact_path.is_file() {
                continue;
            }
            if !self.verify_file(&artifact_path, &cached.sha256).await? {
                continue;
            }
            let artifact = AuthlibInjectorArtifact {
                path: artifact_path,
                version: cached.version,
                build_number: cached.build_number,
            };
            if best
                .as_ref()
                .is_none_or(|best| artifact.build_number > best.build_number)
            {
                best = Some(artifact);
            }
        }
        Ok(best)
    }

    async fn verify_file(
        &self,
        path: &Path,
        expected_sha256: &str,
    ) -> Result<bool, AuthlibInjectorError> {
        let bytes = fs::read(path)
            .await
            .map_err(|source| AuthlibInjectorError::Io {
                path: path.display().to_string(),
                source,
            })?;
        let actual = hex::encode(Sha256::digest(&bytes));
        Ok(actual.eq_ignore_ascii_case(expected_sha256))
    }

    async fn write_cached_manifest(
        &self,
        manifest: &AuthlibInjectorManifest,
    ) -> Result<(), AuthlibInjectorError> {
        let cached = CachedArtifact {
            build_number: manifest.build_number,
            version: manifest.version.clone(),
            sha256: manifest.sha256.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&cached).map_err(AuthlibInjectorError::Json)?;
        let path = self.manifest_path(manifest.build_number);
        write_atomic(&path, &bytes).await
    }

    fn artifact_path(&self, build_number: u64) -> PathBuf {
        self.cache_dir
            .join(format!("authlib-injector-{build_number}.jar"))
    }

    fn manifest_path(&self, build_number: u64) -> PathBuf {
        self.cache_dir
            .join(format!("authlib-injector-{build_number}.json"))
    }
}

#[derive(Debug, Clone, Deserialize)]
struct RawArtifact {
    build_number: u64,
    version: String,
    download_url: String,
    #[serde(default)]
    checksums: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedArtifact {
    build_number: u64,
    version: String,
    sha256: String,
}

#[derive(Debug, Clone)]
struct AuthlibInjectorManifest {
    build_number: u64,
    version: String,
    download_url: String,
    sha256: String,
}

impl AuthlibInjectorManifest {
    fn artifact(&self, path: PathBuf) -> AuthlibInjectorArtifact {
        AuthlibInjectorArtifact {
            path,
            version: self.version.clone(),
            build_number: self.build_number,
        }
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

async fn read_body_limited(
    mut response: reqwest::Response,
    limit: usize,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, AuthlibInjectorError> {
    if let Some(length) = response.content_length()
        && length > limit as u64
    {
        return Err(AuthlibInjectorError::ManifestTooLarge { limit });
    }
    let mut bytes = Vec::new();
    loop {
        let chunk = tokio::select! {
            _ = cancellation.cancelled() => return Err(AuthlibInjectorError::Cancelled),
            chunk = response.chunk() => chunk.map_err(AuthlibInjectorError::Http)?,
        };
        let Some(chunk) = chunk else {
            break;
        };
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(AuthlibInjectorError::ManifestTooLarge { limit });
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), AuthlibInjectorError> {
    let path = path.to_path_buf();
    let parent = path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| AuthlibInjectorError::Io {
            path: path.display().to_string(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "cache path has no parent",
            ),
        })?;
    fs::create_dir_all(&parent)
        .await
        .map_err(|source| AuthlibInjectorError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    let bytes = bytes.to_vec();
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let mut temporary = tempfile::NamedTempFile::new_in(&parent).map_err(|source| {
            AuthlibInjectorError::Io {
                path: path.display().to_string(),
                source,
            }
        })?;
        temporary
            .write_all(&bytes)
            .map_err(|source| AuthlibInjectorError::Io {
                path: temporary.path().display().to_string(),
                source,
            })?;
        temporary
            .persist(&path)
            .map_err(|error| AuthlibInjectorError::Io {
                path: path.display().to_string(),
                source: error.error,
            })?;
        Ok::<_, AuthlibInjectorError>(())
    })
    .await
    .map_err(|error| AuthlibInjectorError::Blocking(error.to_string()))?
}

#[derive(Debug, Error)]
pub enum AuthlibInjectorError {
    #[error("failed to create the authlib-injector HTTP client: {0}")]
    Client(#[source] reqwest::Error),
    #[error("authlib-injector cache I/O failed at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse authlib-injector metadata: {0}")]
    Json(#[source] serde_json::Error),
    #[error("authlib-injector download API returned HTTP {status} from {url}")]
    HttpStatus { url: String, status: StatusCode },
    #[error("authlib-injector download failed: {0}")]
    Http(#[source] reqwest::Error),
    #[error("authlib-injector checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("authlib-injector artifact exceeds {limit} bytes")]
    ArtifactTooLarge { limit: u64 },
    #[error("authlib-injector manifest exceeds {limit} bytes")]
    ManifestTooLarge { limit: usize },
    #[error("no cached authlib-injector artifact is available")]
    NotCached,
    #[error("all authlib-injector sources failed: {attempts}")]
    AllSourcesFailed { attempts: String },
    #[error("authlib-injector download was cancelled")]
    Cancelled,
    #[error("invalid authlib-injector download URL: {0}")]
    InvalidDownloadUrl(String),
    #[error("authlib-injector manifest is missing a valid SHA-256 checksum")]
    MissingChecksum,
    #[error("authlib-injector manifest contains an invalid build number or version")]
    InvalidManifest,
    #[error("authlib-injector cache task failed: {0}")]
    Blocking(String),
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::{Router, extract::State, response::IntoResponse, routing::get};
    use rmcl_net::MirrorPreference;
    use serde_json::json;
    use sha2::{Digest, Sha256};

    use super::*;

    const ARTIFACT: &[u8] = b"fake authlib-injector jar bytes";

    #[derive(Clone)]
    struct MockState {
        artifact_calls: Arc<AtomicUsize>,
        checksum: String,
        base: String,
    }

    async fn spawn_mock() -> (Url, MockState) {
        spawn_mock_with_checksum(hex::encode(Sha256::digest(ARTIFACT))).await
    }

    async fn spawn_mock_with_checksum(checksum: String) -> (Url, MockState) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let base = format!("http://{address}/");
        let state = MockState {
            artifact_calls: Arc::new(AtomicUsize::new(0)),
            checksum,
            base: base.clone(),
        };
        let router = Router::new()
            .route("/artifact/latest.json", get(mock_manifest))
            .route("/artifact/download.jar", get(mock_artifact))
            .with_state(state.clone());
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (Url::parse(&base).unwrap(), state)
    }

    async fn mock_manifest(State(state): State<MockState>) -> impl IntoResponse {
        axum::Json(json!({
            "build_number": 56,
            "version": "1.2.8",
            "download_url": format!("{}artifact/download.jar", state.base),
            "checksums": {"sha256": state.checksum}
        }))
    }

    async fn mock_artifact(State(state): State<MockState>) -> impl IntoResponse {
        state.artifact_calls.fetch_add(1, Ordering::SeqCst);
        ARTIFACT
    }

    fn service(base: &Url, cache: &Path) -> AuthlibInjectorService {
        let manifest = base.join("artifact/latest.json").unwrap();
        AuthlibInjectorService::with_endpoints(
            Arc::new(MirrorHealth::new(MirrorPreference::Official)),
            cache,
            manifest,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn downloads_and_caches_a_verified_artifact() {
        let (base, state) = spawn_mock().await;
        let cache = tempfile::tempdir().unwrap();
        let service = service(&base, cache.path());
        let cancellation = CancellationToken::new();

        let first = service.ensure(&cancellation).await.unwrap();
        assert_eq!(first.build_number, 56);
        assert_eq!(first.version, "1.2.8");
        assert_eq!(state.artifact_calls.load(Ordering::SeqCst), 1);
        assert!(first.path.is_file());

        let second = service.ensure(&cancellation).await.unwrap();
        assert_eq!(second, first);
        assert_eq!(state.artifact_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn checksum_mismatch_is_rejected_and_not_cached() {
        let (base, state) = spawn_mock_with_checksum("0".repeat(64)).await;
        let cache = tempfile::tempdir().unwrap();
        let service = service(&base, cache.path());
        let error = service.ensure(&CancellationToken::new()).await.unwrap_err();
        assert!(matches!(
            error,
            AuthlibInjectorError::AllSourcesFailed { .. }
        ));
        assert_eq!(state.artifact_calls.load(Ordering::SeqCst), 1);
        assert!(!cache.path().join("authlib-injector-56.jar").exists());
    }

    #[tokio::test]
    async fn cached_artifact_is_used_when_the_manifest_is_unavailable() {
        let cache = tempfile::tempdir().unwrap();
        let artifact = cache.path().join("authlib-injector-56.jar");
        tokio::fs::write(&artifact, ARTIFACT).await.unwrap();
        let manifest = CachedArtifact {
            build_number: 56,
            version: "1.2.8".to_owned(),
            sha256: hex::encode(Sha256::digest(ARTIFACT)),
        };
        tokio::fs::write(
            cache.path().join("authlib-injector-56.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .await
        .unwrap();

        let dead = Url::parse("http://127.0.0.1:1/artifact/latest.json").unwrap();
        let service = AuthlibInjectorService::with_endpoints(
            Arc::new(MirrorHealth::new(MirrorPreference::Official)),
            cache.path(),
            dead,
        )
        .unwrap();
        let resolved = service.ensure(&CancellationToken::new()).await.unwrap();
        assert_eq!(resolved.path, artifact);
        assert_eq!(resolved.build_number, 56);
    }
}
