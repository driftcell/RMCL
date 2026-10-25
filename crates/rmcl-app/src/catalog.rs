use std::sync::Arc;

use rmcl_domain::DownloadRequest;
use rmcl_minecraft::VersionManifest;
use rmcl_net::{DownloadConfig, DownloadError, DownloadExecutor};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use crate::{AppError, InstallPhase, VanillaSources};

/// Caches the Mojang version manifest for browser and install entry points.
#[derive(Clone)]
pub struct VersionCatalog {
    sources: VanillaSources,
    downloader: DownloadExecutor,
    cached: Arc<RwLock<Option<Arc<VersionManifest>>>>,
    fetch_lock: Arc<Mutex<()>>,
}

impl VersionCatalog {
    pub fn official(config: DownloadConfig) -> Result<Self, AppError> {
        Self::new(config, VanillaSources::official()?)
    }

    pub fn new(config: DownloadConfig, sources: VanillaSources) -> Result<Self, AppError> {
        Ok(Self {
            sources,
            downloader: DownloadExecutor::new(config)?,
            cached: Arc::new(RwLock::new(None)),
            fetch_lock: Arc::new(Mutex::new(())),
        })
    }

    pub async fn cached(&self) -> Option<Arc<VersionManifest>> {
        self.cached.read().await.clone()
    }

    /// Returns the manifest, downloading it when `refresh` is set or no cache exists.
    pub async fn manifest(
        &self,
        refresh: bool,
        cancellation: CancellationToken,
    ) -> Result<Arc<VersionManifest>, AppError> {
        if !refresh && let Some(manifest) = self.cached().await {
            return Ok(manifest);
        }

        let _fetch = self.fetch_lock.lock().await;
        if !refresh && let Some(manifest) = self.cached().await {
            return Ok(manifest);
        }
        check_cancelled(&cancellation)?;

        let temporary = tempfile::tempdir().map_err(|source| AppError::Io {
            operation: "create version manifest temporary directory",
            path: std::env::temp_dir(),
            source,
        })?;
        let destination = temporary.path().join("version_manifest_v2.json");
        let request = DownloadRequest {
            sources: vec![self.sources.manifest_url.clone()],
            destination: destination.clone(),
            sha1: None,
            size: None,
        };
        self.downloader
            .download_file(&request, cancellation.clone(), None)
            .await
            .map_err(|source| catalog_download_error(source, &cancellation))?;
        check_cancelled(&cancellation)?;

        let bytes = tokio::fs::read(&destination)
            .await
            .map_err(|source| AppError::Io {
                operation: "read version manifest",
                path: destination.clone(),
                source,
            })?;
        let json = String::from_utf8(bytes).map_err(|source| AppError::JsonUtf8 {
            path: destination,
            source,
        })?;
        let manifest = Arc::new(VersionManifest::from_json(&json)?);
        *self.cached.write().await = Some(manifest.clone());
        Ok(manifest)
    }
}

fn catalog_download_error(source: DownloadError, cancellation: &CancellationToken) -> AppError {
    if cancellation.is_cancelled() || matches!(&source, DownloadError::Cancelled) {
        AppError::Cancelled
    } else {
        AppError::Download {
            phase: InstallPhase::ResolvingVersion,
            source,
        }
    }
}

fn check_cancelled(cancellation: &CancellationToken) -> Result<(), AppError> {
    if cancellation.is_cancelled() {
        Err(AppError::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use axum::{Router, routing::get};
    use url::Url;

    use super::*;

    struct Fixture {
        base: Url,
        hits: Arc<AtomicUsize>,
        server: tokio::task::JoinHandle<()>,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    async fn fixture(body: &'static str) -> Fixture {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let base = Url::parse(&format!("http://{address}/")).unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let router = Router::new().route(
            "/manifest.json",
            get({
                let hits = hits.clone();
                move || {
                    let hits = hits.clone();
                    async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        body
                    }
                }
            }),
        );
        let server = tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, router).await {
                eprintln!("catalog test server failed: {error}");
            }
        });
        Fixture { base, hits, server }
    }

    fn catalog(fixture: &Fixture) -> VersionCatalog {
        let sources = VanillaSources {
            manifest_url: fixture.base.join("manifest.json").unwrap(),
            asset_object_base: fixture.base.join("objects/").unwrap(),
        };
        VersionCatalog::new(config(), sources).unwrap()
    }

    fn config() -> DownloadConfig {
        DownloadConfig {
            max_concurrency: 1,
            max_concurrency_per_host: 1,
            max_attempts_per_source: 1,
            connect_timeout: Duration::from_secs(1),
            request_timeout: Duration::from_secs(2),
            initial_retry_delay: Duration::from_millis(1),
            max_retry_delay: Duration::from_millis(5),
        }
    }

    fn manifest(id: &str) -> &'static str {
        Box::leak(
            format!(
                r#"{{"latest":{{"release":"{id}"}},"versions":[{{"id":"{id}","type":"release","url":"https://example.invalid/{id}.json"}}]}}"#
            )
            .into_boxed_str(),
        )
    }

    #[tokio::test]
    async fn caches_until_a_refresh_is_requested() {
        let fixture = fixture(manifest("1.21.1")).await;
        let catalog = catalog(&fixture);

        let first = catalog
            .manifest(false, CancellationToken::new())
            .await
            .unwrap();
        let second = catalog
            .manifest(false, CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(first.find("1.21.1").unwrap().id, "1.21.1");
        assert_eq!(Arc::as_ptr(&first), Arc::as_ptr(&second));
        assert_eq!(fixture.hits.load(Ordering::SeqCst), 1);

        catalog
            .manifest(true, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(fixture.hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn cancellation_before_fetch_is_not_a_network_error() {
        let fixture = fixture(manifest("1.21.1")).await;
        let catalog = catalog(&fixture);
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let error = catalog.manifest(false, cancellation).await.unwrap_err();

        assert!(matches!(error, AppError::Cancelled));
        assert_eq!(fixture.hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn invalid_manifest_is_reported_as_metadata() {
        let fixture = fixture("not json").await;
        let catalog = catalog(&fixture);

        assert!(matches!(
            catalog.manifest(false, CancellationToken::new()).await,
            Err(AppError::Metadata(_))
        ));
    }
}
