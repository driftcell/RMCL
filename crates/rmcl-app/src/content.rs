use std::{collections::HashSet, path::PathBuf};

use rmcl_content::{
    ConflictPolicy, ContentKind, ContentLibrary, LocalContent, ModrinthClient, RemoteFile,
    RemoteVersion, delete_content, import_file, scan_library, set_content_enabled,
};
use rmcl_domain::DownloadRequest;
use rmcl_instance::{InstanceLayout, MinecraftLayout, RepositoryPath};
use rmcl_net::{DownloadConfig, DownloadExecutor, DownloadStatus, ProgressStage, progress_channel};
use sha1::{Digest, Sha1};
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
    pub completed_files: usize,
    pub total_files: usize,
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

/// One primary remote version plus its required Modrinth dependencies.
#[derive(Debug, Clone)]
pub struct ContentGraphInstallRequest {
    pub instance: InstanceLayout,
    pub kind: ContentKind,
    pub version: RemoteVersion,
    pub display_name: String,
    pub conflict: ConflictPolicy,
    pub game_version: String,
    pub loader: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ContentBatchOutcome {
    pub items: Vec<ContentInstallOutcome>,
    pub downloaded_bytes: u64,
    pub skipped_files: usize,
}

/// Resolves the primary file and all required Modrinth dependencies into an
/// ordered install batch. Dependencies come first so a failure cannot leave a
/// mod installed without its required libraries.
///
/// Files already present with the same SHA-1 (including disabled mods) are
/// skipped. Dependency cycles and duplicate version IDs are collapsed; more
/// than 32 distinct versions is treated as an unsafe graph.
pub async fn resolve_content_install_batch(
    client: &ModrinthClient,
    request: &ContentGraphInstallRequest,
    cancellation: &CancellationToken,
) -> Result<Vec<ContentInstallRequest>, AppError> {
    installable_file(&request.version, request.kind)?;
    let mut discovered = HashSet::new();
    discovered.insert(request.version.id.clone());
    let mut order = Vec::new();
    let mut stack = vec![(request.version.clone(), false)];
    while let Some((version, expanded)) = stack.pop() {
        if expanded {
            order.push(version);
            continue;
        }
        if discovered.len() > 32 {
            return Err(AppError::ContentDependency {
                message: "required dependency graph exceeds 32 versions".to_owned(),
            });
        }
        let mut dependencies = Vec::new();
        if request.kind == ContentKind::Mod {
            for dependency in version.required_dependencies() {
                let resolved = resolve_dependency(
                    client,
                    dependency,
                    request.kind,
                    &request.game_version,
                    request.loader.as_deref(),
                    cancellation,
                )
                .await?;
                installable_file(&resolved, request.kind)?;
                if discovered.insert(resolved.id.clone()) {
                    dependencies.push(resolved);
                }
            }
        }
        stack.push((version, true));
        for dependency in dependencies.into_iter().rev() {
            stack.push((dependency, false));
        }
    }

    let mut requests = Vec::new();
    for version in order {
        let file = installable_file(&version, request.kind)?;
        if content_already_installed(&request.instance, request.kind, &file) {
            continue;
        }
        let display_name = if version.id == request.version.id {
            request.display_name.clone()
        } else if version.name.trim().is_empty() {
            file.filename.clone()
        } else {
            version.name.clone()
        };
        requests.push(ContentInstallRequest {
            instance: request.instance.clone(),
            kind: request.kind,
            file,
            display_name,
            conflict: ConflictPolicy::Rename,
        });
    }
    Ok(requests)
}

async fn resolve_dependency(
    client: &ModrinthClient,
    dependency: &rmcl_content::RemoteDependency,
    kind: ContentKind,
    game_version: &str,
    loader: Option<&str>,
    cancellation: &CancellationToken,
) -> Result<RemoteVersion, AppError> {
    if let Some(version_id) = dependency.version_id.as_deref() {
        return Ok(client.version(version_id, cancellation).await?);
    }
    if let Some(project_id) = dependency.project_id.as_deref() {
        let loader = (kind == ContentKind::Mod).then_some(loader).flatten();
        let versions = client
            .project_versions(project_id, Some(game_version), loader, cancellation)
            .await?;
        return versions
            .into_iter()
            .find(|version| version.installable_file(kind).is_some())
            .ok_or_else(|| AppError::ContentDependency {
                message: format!(
                    "required dependency project `{project_id}` has no compatible version for Minecraft {game_version}"
                ),
            });
    }
    Err(AppError::ContentDependency {
        message: format!(
            "required dependency `{}` has neither a version nor a project ID",
            dependency.file_name.as_deref().unwrap_or("<unknown>")
        ),
    })
}

fn installable_file(version: &RemoteVersion, kind: ContentKind) -> Result<RemoteFile, AppError> {
    let file =
        version
            .installable_file(kind)
            .cloned()
            .ok_or_else(|| AppError::ContentDependency {
                message: format!(
                    "`{}` has no installable {} file",
                    version.name,
                    kind.label()
                ),
            })?;
    if file.sha1.is_none() {
        return Err(AppError::ContentDependency {
            message: format!("`{}` has no SHA-1", file.filename),
        });
    }
    Ok(file)
}

fn content_already_installed(
    instance: &InstanceLayout,
    kind: ContentKind,
    file: &RemoteFile,
) -> bool {
    let Some(expected) = file.sha1 else {
        return false;
    };
    let directory = kind.directory(instance);
    let path = directory.join(&file.filename);
    let disabled = directory.join(format!("{}.disabled", file.filename));
    let candidate = if path.is_file() {
        path
    } else if disabled.is_file() {
        disabled
    } else {
        return false;
    };
    let Ok(mut input) = std::fs::File::open(candidate) else {
        return false;
    };
    let mut hasher = Sha1::new();
    if std::io::copy(&mut input, &mut hasher).is_err() {
        return false;
    }
    let digest: [u8; 20] = hasher.finalize().into();
    digest == *expected.as_bytes()
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
        request: ContentInstallRequest,
        cancellation: CancellationToken,
        events: Option<ContentInstallEventSender>,
    ) -> Result<ContentInstallOutcome, AppError> {
        self.install_inner(request, cancellation, events, 0, 1, true)
            .await
    }

    /// Installs files sequentially under one cancellation token and one
    /// progress stream. Dependency order is preserved exactly as provided.
    pub async fn install_batch(
        &self,
        requests: Vec<ContentInstallRequest>,
        cancellation: CancellationToken,
        events: Option<ContentInstallEventSender>,
    ) -> Result<ContentBatchOutcome, AppError> {
        if requests.is_empty() {
            return Err(AppError::InvalidRequest(
                "content install batch is empty".to_owned(),
            ));
        }
        let total_files = requests.len();
        let mut items = Vec::with_capacity(total_files);
        let mut downloaded_bytes = 0;
        let mut skipped_files = 0;
        for (index, request) in requests.into_iter().enumerate() {
            check_cancelled(&cancellation)?;
            let (sender, mut receiver) = mpsc::channel(128);
            let install = self.install_inner(
                request,
                cancellation.clone(),
                Some(sender),
                index,
                total_files,
                false,
            );
            let forward = async {
                while let Some(event) = receiver.recv().await {
                    emit_content(&events, event).await;
                }
            };
            let (result, ()) = tokio::join!(install, forward);
            let outcome = result?;
            downloaded_bytes += outcome.downloaded_bytes;
            skipped_files += usize::from(outcome.skipped);
            items.push(outcome);
        }
        emit_content_phase(&events, ContentInstallPhase::Completed).await;
        Ok(ContentBatchOutcome {
            items,
            downloaded_bytes,
            skipped_files,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn install_inner(
        &self,
        mut request: ContentInstallRequest,
        cancellation: CancellationToken,
        events: Option<ContentInstallEventSender>,
        completed_files: usize,
        total_files: usize,
        emit_completed: bool,
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
                        completed_files,
                        total_files,
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
        if emit_completed {
            emit_content_phase(&events, ContentInstallPhase::Completed).await;
        }
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
