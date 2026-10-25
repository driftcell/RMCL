use std::path::PathBuf;

use rmcl_content::{
    ConflictPolicy, ContentKind, ContentLibrary, LocalContent, RemoteFile, delete_content,
    import_file, scan_library, set_content_enabled,
};
use rmcl_domain::DownloadRequest;
use rmcl_instance::{InstanceLayout, MinecraftLayout, RepositoryPath};
use rmcl_net::{DownloadConfig, DownloadExecutor, DownloadStatus, ProgressStage, progress_channel};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::AppError;

/// Scans all local content directories of one instance.
pub async fn scan_instance_content(instance: InstanceLayout) -> Result<ContentLibrary, AppError> {
    tokio::task::spawn_blocking(move || scan_library(&instance))
        .await
        .map_err(|error| AppError::BlockingTask(error.to_string()))
}

/// Imports one local file into the selected instance content directory.
pub async fn import_instance_content(
    instance: InstanceLayout,
    kind: ContentKind,
    source: PathBuf,
    policy: ConflictPolicy,
) -> Result<LocalContent, AppError> {
    tokio::task::spawn_blocking(move || import_file(&instance, kind, source, policy))
        .await
        .map_err(|error| AppError::BlockingTask(error.to_string()))?
        .map_err(AppError::Content)
}

/// Deletes one local content entry from an instance.
pub async fn delete_instance_content(
    instance: InstanceLayout,
    kind: ContentKind,
    relative_path: RepositoryPath,
) -> Result<(), AppError> {
    tokio::task::spawn_blocking(move || delete_content(&instance, kind, &relative_path))
        .await
        .map_err(|error| AppError::BlockingTask(error.to_string()))?
        .map_err(AppError::Content)
}

/// Enables or disables one mod or resource pack.
pub async fn set_instance_content_enabled(
    instance: InstanceLayout,
    kind: ContentKind,
    relative_path: RepositoryPath,
    enabled: bool,
) -> Result<LocalContent, AppError> {
    tokio::task::spawn_blocking(move || {
        set_content_enabled(&instance, kind, &relative_path, enabled)
    })
    .await
    .map_err(|error| AppError::BlockingTask(error.to_string()))?
    .map_err(AppError::Content)
}

/// One remote file install request owned by the task manager.
#[derive(Debug, Clone)]
pub struct ContentInstallRequest {
    pub instance: InstanceLayout,
    pub kind: ContentKind,
    pub file: RemoteFile,
    pub display_name: String,
    pub conflict: ConflictPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentInstallPhase {
    Preparing,
    Downloading,
    Installing,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentDownloadProgress {
    pub phase: ContentInstallPhase,
    pub stage: ProgressStage,
    pub current: PathBuf,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentInstallEvent {
    Phase(ContentInstallPhase),
    Download(ContentDownloadProgress),
}

pub type ContentInstallEventSender = mpsc::Sender<ContentInstallEvent>;

#[derive(Debug, Clone)]
pub struct ContentInstallOutcome {
    pub content: LocalContent,
    pub downloaded_bytes: u64,
    pub skipped: bool,
}

/// Downloads one remote content file into an instance.
///
/// The file is downloaded to a temporary directory with the existing download
/// executor, verified by SHA-1, and only then published into the content
/// directory through the atomic local import path.
#[derive(Clone)]
pub struct ContentInstaller {
    downloader: DownloadExecutor,
}

impl ContentInstaller {
    pub fn official(config: DownloadConfig) -> Result<Self, AppError> {
        Ok(Self {
            downloader: DownloadExecutor::new(config)?,
        })
    }

    pub async fn install(
        &self,
        mut request: ContentInstallRequest,
        cancellation: CancellationToken,
        events: Option<ContentInstallEventSender>,
    ) -> Result<ContentInstallOutcome, AppError> {
        check_cancelled(&cancellation)?;
        request.instance = absolute_instance(&request.instance)?;
        validate_remote_file_name(&request.file.filename)?;
        if !request.kind.accepts_file_name(&request.file.filename) {
            return Err(AppError::InvalidRequest(format!(
                "remote file `{}` is not a supported {} file",
                request.file.filename,
                request.kind.label()
            )));
        }
        if !matches!(request.file.url.scheme(), "http" | "https") {
            return Err(AppError::InvalidRequest(format!(
                "remote content URL `{}` is not HTTP(S)",
                request.file.url
            )));
        }
        let sha1 = request.file.sha1.ok_or_else(|| {
            AppError::InvalidRequest("remote content file has no SHA-1".to_owned())
        })?;

        emit_content_phase(&events, ContentInstallPhase::Preparing).await;
        let temporary = tempfile::tempdir().map_err(|source| AppError::Io {
            operation: "create content download temporary directory",
            path: std::env::temp_dir(),
            source,
        })?;
        let destination = temporary.path().join(&request.file.filename);
        let download = DownloadRequest {
            sources: vec![request.file.url.clone()],
            destination: destination.clone(),
            sha1: Some(sha1),
            size: request.file.size,
        };
        let (progress, mut receiver) = progress_channel(128);
        let download_future =
            self.downloader
                .download_file(&download, cancellation.clone(), Some(progress));
        let forward = async {
            while let Some(event) = receiver.recv().await {
                emit_content(
                    &events,
                    ContentInstallEvent::Download(ContentDownloadProgress {
                        phase: ContentInstallPhase::Downloading,
                        stage: event.stage,
                        current: event.destination,
                        downloaded_bytes: event.downloaded_bytes,
                        total_bytes: event.total_bytes,
                        detail: event.detail,
                    }),
                )
                .await;
            }
        };
        emit_content_phase(&events, ContentInstallPhase::Downloading).await;
        let (status, ()) = tokio::join!(download_future, forward);
        let status = status.map_err(|source| content_download_error(source, &cancellation))?;
        check_cancelled(&cancellation)?;

        emit_content_phase(&events, ContentInstallPhase::Installing).await;
        let instance = request.instance.clone();
        let kind = request.kind;
        let conflict = request.conflict;
        let import_path = destination.clone();
        let content = tokio::task::spawn_blocking(move || {
            import_file(&instance, kind, &import_path, conflict)
        })
        .await
        .map_err(|error| AppError::BlockingTask(error.to_string()))?
        .map_err(AppError::Content)?;
        let (downloaded_bytes, skipped) = match status {
            DownloadStatus::Downloaded { bytes } => (bytes, false),
            DownloadStatus::Skipped { bytes } => (bytes, true),
        };
        emit_content_phase(&events, ContentInstallPhase::Completed).await;
        Ok(ContentInstallOutcome {
            content,
            downloaded_bytes,
            skipped,
        })
    }
}

pub(crate) fn absolute_instance(instance: &InstanceLayout) -> Result<InstanceLayout, AppError> {
    let root =
        std::path::absolute(instance.repository().root()).map_err(|source| AppError::Io {
            operation: "resolve Minecraft root",
            path: instance.repository().root().to_path_buf(),
            source,
        })?;
    let repository = MinecraftLayout::new(root);
    Ok(repository.instance(instance.id().clone(), instance.isolation()))
}

fn validate_remote_file_name(name: &str) -> Result<(), AppError> {
    if name.trim().is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name == "."
        || name == ".."
    {
        return Err(AppError::InvalidRequest(format!(
            "remote content filename `{name}` is not a single safe path component"
        )));
    }
    RepositoryPath::new(name).map_err(|error| AppError::Content(error.into()))?;
    Ok(())
}

fn content_download_error(
    source: rmcl_net::DownloadError,
    cancellation: &CancellationToken,
) -> AppError {
    if cancellation.is_cancelled() || matches!(&source, rmcl_net::DownloadError::Cancelled) {
        AppError::Cancelled
    } else {
        AppError::ContentDownload { source }
    }
}

fn check_cancelled(cancellation: &CancellationToken) -> Result<(), AppError> {
    if cancellation.is_cancelled() {
        Err(AppError::Cancelled)
    } else {
        Ok(())
    }
}

async fn emit_content_phase(
    events: &Option<ContentInstallEventSender>,
    phase: ContentInstallPhase,
) {
    emit_content(events, ContentInstallEvent::Phase(phase)).await;
}

async fn emit_content(events: &Option<ContentInstallEventSender>, event: ContentInstallEvent) {
    if let Some(events) = events {
        let _ = events.send(event).await;
    }
}
