use std::{path::PathBuf, sync::Arc};

use rmcl_java::{
    JavaDetector, JavaRequirement, JavaRuntime, MojangRuntimeInstaller, UrlPolicy, select_java,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::AppError;

#[derive(Clone)]
pub struct JavaServiceConfig {
    pub managed_roots: Vec<PathBuf>,
    pub store_root: PathBuf,
    pub include_java_home: bool,
    pub include_path: bool,
    pub auto_install: bool,
    pub runtime_index_url: Option<Url>,
    pub url_policy: UrlPolicy,
}

impl JavaServiceConfig {
    pub fn new(store_root: impl Into<PathBuf>) -> Self {
        Self {
            managed_roots: Vec::new(),
            store_root: store_root.into(),
            include_java_home: true,
            include_path: true,
            auto_install: true,
            runtime_index_url: None,
            url_policy: UrlPolicy::OfficialOnly,
        }
    }
}

#[derive(Clone)]
pub struct JavaService {
    detector: JavaDetector,
    installer: Arc<MojangRuntimeInstaller>,
    auto_install: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JavaEvent {
    Checking,
    Installing,
}

pub type JavaEventSender = mpsc::Sender<JavaEvent>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JavaSelectionOrigin {
    Detected,
    Installed,
    Custom,
}

#[derive(Debug, Clone)]
pub struct JavaSelection {
    pub runtime: JavaRuntime,
    pub origin: JavaSelectionOrigin,
}

impl JavaService {
    pub fn new(config: JavaServiceConfig) -> Result<Self, AppError> {
        let store_root = absolute(&config.store_root)?;
        let mut managed_roots = config
            .managed_roots
            .iter()
            .map(absolute)
            .collect::<Result<Vec<_>, _>>()?;
        managed_roots.push(store_root.clone());
        managed_roots.sort();
        managed_roots.dedup();
        let detector = JavaDetector::new(managed_roots)
            .with_java_home(config.include_java_home)
            .with_path(config.include_path);
        let mut installer = MojangRuntimeInstaller::new(store_root)?;
        if let Some(url) = config.runtime_index_url {
            installer = installer.with_index_url(url.to_string());
        }
        installer = installer.with_url_policy(config.url_policy);
        Ok(Self {
            detector,
            installer: Arc::new(installer),
            auto_install: config.auto_install,
        })
    }

    /// Probes every configured candidate. Used by the instance Java picker and
    /// by diagnostics; callers that need a launch-ready runtime should use
    /// [`Self::ensure`] instead.
    pub async fn discover(&self) -> rmcl_java::JavaDiscovery {
        self.detector.discover().await
    }

    /// Probes a user-supplied executable without adding it to automatic
    /// discovery. The result still has to satisfy the game's Java requirement
    /// when the instance is launched.
    pub async fn probe(&self, executable: PathBuf) -> Result<JavaRuntime, AppError> {
        rmcl_java::probe(executable, rmcl_java::JavaRuntimeSource::Custom)
            .await
            .map_err(AppError::Java)
    }

    pub async fn ensure(
        &self,
        requirement: &JavaRequirement,
        cancellation: CancellationToken,
        events: Option<JavaEventSender>,
    ) -> Result<JavaSelection, AppError> {
        emit(&events, JavaEvent::Checking).await;
        check_cancelled(&cancellation)?;
        let discovery = self.detector.discover().await;
        check_cancelled(&cancellation)?;
        if let Some(runtime) = select_java(&discovery.runtimes, requirement) {
            return Ok(JavaSelection {
                runtime: runtime.clone(),
                origin: JavaSelectionOrigin::Detected,
            });
        }
        if !self.auto_install {
            return Err(AppError::JavaNotFound {
                requirement: Box::new(requirement.clone()),
                failures: discovery
                    .failures
                    .iter()
                    .map(|failure| format!("{}: {}", failure.executable.display(), failure.error))
                    .collect(),
            });
        }

        emit(&events, JavaEvent::Installing).await;
        let runtime = self
            .installer
            .install(requirement, cancellation.clone())
            .await
            .map_err(|error| java_error(error, &cancellation))?;
        check_cancelled(&cancellation)?;
        Ok(JavaSelection {
            runtime,
            origin: JavaSelectionOrigin::Installed,
        })
    }
}

fn absolute(path: &PathBuf) -> Result<PathBuf, AppError> {
    std::path::absolute(path).map_err(|source| AppError::Io {
        operation: "resolve Java path",
        path: path.clone(),
        source,
    })
}

fn java_error(error: rmcl_java::JavaError, cancellation: &CancellationToken) -> AppError {
    if cancellation.is_cancelled()
        || matches!(
            &error,
            rmcl_java::JavaError::Download(rmcl_net::DownloadError::Cancelled)
        )
    {
        AppError::Cancelled
    } else {
        AppError::Java(error)
    }
}

fn check_cancelled(cancellation: &CancellationToken) -> Result<(), AppError> {
    if cancellation.is_cancelled() {
        Err(AppError::Cancelled)
    } else {
        Ok(())
    }
}

async fn emit(events: &Option<JavaEventSender>, event: JavaEvent) {
    if let Some(events) = events {
        let _ = events.send(event).await;
    }
}
