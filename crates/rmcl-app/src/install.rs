use std::{collections::HashSet, io::Write, path::Path};

use rmcl_archive::{ArchiveExtractor, ExtractionError, ExtractionReport};
use rmcl_domain::DownloadRequest;
use rmcl_install::{InstallPlan, PlannedFileKind, plan_asset_objects_with_base, plan_vanilla};
use rmcl_instance::{MinecraftLayout, VersionId};
use rmcl_minecraft::{AssetCatalog, GameVersion, VersionManifest, VersionSummary};
use rmcl_net::{DownloadConfig, DownloadExecutor, DownloadReport, ProgressStage, progress_channel};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::{
    AppError, DownloadSummary, InstallDownloadFailure, InstallDownloadProgress, InstallEvent,
    InstallEventSender, InstallOutcome, InstallPhase, VanillaInstallRequest, VanillaSources,
    VersionSource,
};

#[derive(Clone)]
pub struct VanillaInstaller {
    sources: VanillaSources,
    downloader: DownloadExecutor,
    extractor: ArchiveExtractor,
}

impl VanillaInstaller {
    pub fn official(config: DownloadConfig) -> Result<Self, AppError> {
        Self::new(config, VanillaSources::official()?)
    }

    pub fn new(config: DownloadConfig, sources: VanillaSources) -> Result<Self, AppError> {
        Ok(Self {
            sources,
            downloader: DownloadExecutor::new(config)?,
            extractor: ArchiveExtractor::default(),
        })
    }

    /// Installs one standalone Vanilla version and returns all data needed for a later launch.
    ///
    /// Callers must serialize installations that target the same repository and version.
    pub async fn install(
        &self,
        request: VanillaInstallRequest,
        cancellation: CancellationToken,
        events: Option<InstallEventSender>,
    ) -> Result<InstallOutcome, AppError> {
        check_cancelled(&cancellation)?;
        let root = std::path::absolute(&request.minecraft_root).map_err(|source| AppError::Io {
            operation: "resolve Minecraft root",
            path: request.minecraft_root.clone(),
            source,
        })?;
        let layout = MinecraftLayout::new(root);
        let id = VersionId::new(request.version_id.clone())?;
        let version_json_path = layout.version_json_path(&id);
        let version_json = self
            .resolve_version_json(&request, &version_json_path, &cancellation, &events)
            .await?;
        let version_json = json_string(version_json, &version_json_path)?;
        let version = GameVersion::from_json(&version_json)?;
        if version.id != request.version_id {
            return Err(AppError::VersionIdMismatch {
                expected: request.version_id.clone(),
                actual: version.id.clone(),
            });
        }

        emit_phase(&events, InstallPhase::PlanningBase).await;
        check_cancelled(&cancellation)?;
        let base_plan = plan_vanilla(&layout, &version, &request.platform)?;
        emit_phase(&events, InstallPhase::DownloadingBase).await;
        let mut downloads = self
            .download_plan(
                &base_plan,
                InstallPhase::DownloadingBase,
                &cancellation,
                &events,
            )
            .await?;

        emit_phase(&events, InstallPhase::ReadingAssetIndex).await;
        check_cancelled(&cancellation)?;
        let asset_index = base_plan
            .files
            .iter()
            .find(|file| file.kind == PlannedFileKind::AssetIndex)
            .ok_or_else(|| AppError::InvalidRequest("base plan has no asset index".to_owned()))?;
        let catalog_json = read_bytes(&asset_index.destination, "read asset index").await?;
        let catalog_json = json_string(catalog_json, &asset_index.destination)?;
        let catalog = AssetCatalog::from_json(&catalog_json)?;

        emit_phase(&events, InstallPhase::PlanningAssets).await;
        let asset_plan =
            plan_asset_objects_with_base(&layout, &catalog, &self.sources.asset_object_base)?;
        emit_phase(&events, InstallPhase::DownloadingAssets).await;
        let asset_downloads = self
            .download_plan(
                &asset_plan,
                InstallPhase::DownloadingAssets,
                &cancellation,
                &events,
            )
            .await?;
        downloads.extend(asset_downloads);

        emit_phase(&events, InstallPhase::MergingNatives).await;
        let natives = merge_natives(
            self.extractor.clone(),
            base_plan.extractions.clone(),
            cancellation.clone(),
            events.clone(),
        )
        .await?;
        check_cancelled(&cancellation)?;
        emit_phase(&events, InstallPhase::Completed).await;
        Ok(InstallOutcome {
            layout,
            platform: request.platform,
            version,
            version_json_path,
            base_plan,
            asset_plan,
            downloads,
            natives,
        })
    }

    async fn resolve_version_json(
        &self,
        request: &VanillaInstallRequest,
        destination: &Path,
        cancellation: &CancellationToken,
        events: &Option<InstallEventSender>,
    ) -> Result<Vec<u8>, AppError> {
        match &request.source {
            VersionSource::Manifest => {
                emit_phase(events, InstallPhase::ResolvingVersion).await;
                let summary = self
                    .resolve_remote_version(&request.version_id, cancellation)
                    .await?;
                emit_phase(events, InstallPhase::PersistingVersion).await;
                let request = DownloadRequest {
                    sources: vec![Url::parse(&summary.url)?],
                    destination: destination.to_path_buf(),
                    sha1: summary.sha1.as_deref().map(str::parse).transpose()?,
                    size: None,
                };
                self.downloader
                    .download_file(&request, cancellation.clone(), None)
                    .await
                    .map_err(|source| {
                        download_error(InstallPhase::PersistingVersion, source, cancellation)
                    })?;
                check_cancelled(cancellation)?;
                read_bytes(destination, "read version JSON").await
            }
            VersionSource::LocalJson(source) => {
                emit_phase(events, InstallPhase::PersistingVersion).await;
                let bytes = read_bytes(source, "read local version JSON").await?;
                let json = json_string(bytes.clone(), source)?;
                let version = GameVersion::from_json(&json)?;
                if version.id != request.version_id {
                    return Err(AppError::VersionIdMismatch {
                        expected: request.version_id.clone(),
                        actual: version.id,
                    });
                }
                write_changed_file(destination, &bytes, cancellation.clone()).await?;
                check_cancelled(cancellation)?;
                Ok(bytes)
            }
            VersionSource::Existing => {
                emit_phase(events, InstallPhase::ReadingVersion).await;
                read_bytes(destination, "read existing version JSON").await
            }
        }
    }

    async fn resolve_remote_version(
        &self,
        version_id: &str,
        cancellation: &CancellationToken,
    ) -> Result<VersionSummary, AppError> {
        let temporary = tempfile::tempdir().map_err(|source| AppError::Io {
            operation: "create metadata temporary directory",
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
            .map_err(|source| {
                download_error(InstallPhase::ResolvingVersion, source, cancellation)
            })?;
        check_cancelled(cancellation)?;
        let manifest = read_bytes(&destination, "read version manifest").await?;
        let manifest = json_string(manifest, &destination)?;
        let manifest = VersionManifest::from_json(&manifest)?;
        manifest
            .find(version_id)
            .cloned()
            .ok_or_else(|| AppError::VersionNotFound(version_id.to_owned()))
    }

    async fn download_plan(
        &self,
        plan: &InstallPlan,
        phase: InstallPhase,
        cancellation: &CancellationToken,
        events: &Option<InstallEventSender>,
    ) -> Result<DownloadSummary, AppError> {
        check_cancelled(cancellation)?;
        let total_files = plan.files.len();
        let requests = plan.files.iter().map(DownloadRequest::from).collect();
        let (progress, mut receiver) = progress_channel(128);
        let download = self
            .downloader
            .download_all(requests, cancellation.clone(), Some(progress));
        let collect = async {
            let mut completed_files = HashSet::new();
            while let Some(event) = receiver.recv().await {
                if matches!(
                    event.stage,
                    ProgressStage::Completed
                        | ProgressStage::Skipped
                        | ProgressStage::Failed
                        | ProgressStage::Cancelled
                ) {
                    completed_files.insert(event.destination.clone());
                }
                emit(
                    events,
                    InstallEvent::Download(InstallDownloadProgress {
                        phase,
                        stage: event.stage,
                        current: event.destination,
                        completed_files: completed_files.len(),
                        total_files,
                        downloaded_bytes: event.downloaded_bytes,
                        total_bytes: event.total_bytes,
                        detail: event.detail,
                    }),
                )
                .await;
            }
        };
        let (report, ()) = tokio::join!(download, collect);
        validate_download_report(report, phase, cancellation)
    }
}

fn download_error(
    phase: InstallPhase,
    source: rmcl_net::DownloadError,
    cancellation: &CancellationToken,
) -> AppError {
    if cancellation.is_cancelled() || matches!(&source, rmcl_net::DownloadError::Cancelled) {
        AppError::Cancelled
    } else {
        AppError::Download { phase, source }
    }
}

fn validate_download_report(
    report: DownloadReport,
    phase: InstallPhase,
    cancellation: &CancellationToken,
) -> Result<DownloadSummary, AppError> {
    if report.cancelled || cancellation.is_cancelled() {
        return Err(AppError::Cancelled);
    }
    if !report.failures.is_empty() {
        return Err(AppError::DownloadBatch {
            phase,
            failures: report
                .failures
                .iter()
                .map(|failure| InstallDownloadFailure {
                    destination: failure.request.destination.clone(),
                    message: failure.error.to_string(),
                })
                .collect(),
        });
    }
    Ok(DownloadSummary::from_report(&report))
}

async fn merge_natives(
    extractor: ArchiveExtractor,
    extractions: Vec<rmcl_domain::ExtractionPlan>,
    cancellation: CancellationToken,
    events: Option<InstallEventSender>,
) -> Result<Vec<ExtractionReport>, AppError> {
    if extractions.is_empty() {
        return Ok(Vec::new());
    }
    tokio::task::spawn_blocking(move || {
        let total_archives = extractions.len();
        let mut reports = Vec::with_capacity(total_archives);
        for (index, extraction) in extractions.into_iter().enumerate() {
            check_cancelled(&cancellation)?;
            let current = extraction.source.clone();
            let report = extractor
                .merge_zip(&extraction, cancellation.clone(), None)
                .map_err(|source| match source {
                    ExtractionError::Cancelled => AppError::Cancelled,
                    source => AppError::Extraction {
                        path: current.clone(),
                        source,
                    },
                })?;
            reports.push(report);
            emit_blocking(
                &events,
                InstallEvent::Natives {
                    current,
                    completed_archives: index + 1,
                    total_archives,
                },
            );
        }
        Ok(reports)
    })
    .await
    .map_err(|error| AppError::BlockingTask(error.to_string()))?
}

async fn write_changed_file(
    destination: &Path,
    bytes: &[u8],
    cancellation: CancellationToken,
) -> Result<(), AppError> {
    match tokio::fs::read(destination).await {
        Ok(existing) if existing == bytes => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(AppError::Io {
                operation: "read existing version JSON",
                path: destination.to_path_buf(),
                source,
            });
        }
    }
    check_cancelled(&cancellation)?;
    let destination = destination.to_path_buf();
    let bytes = bytes.to_vec();
    tokio::task::spawn_blocking(move || {
        let parent = destination.parent().ok_or_else(|| AppError::Io {
            operation: "find version JSON parent",
            path: destination.clone(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "version JSON has no parent directory",
            ),
        })?;
        std::fs::create_dir_all(parent).map_err(|source| AppError::Io {
            operation: "create version JSON directory",
            path: parent.to_path_buf(),
            source,
        })?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(parent).map_err(|source| AppError::Io {
                operation: "create version JSON temporary file",
                path: parent.to_path_buf(),
                source,
            })?;
        temporary.write_all(&bytes).map_err(|source| AppError::Io {
            operation: "write version JSON temporary file",
            path: temporary.path().to_path_buf(),
            source,
        })?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|source| AppError::Io {
                operation: "sync version JSON temporary file",
                path: temporary.path().to_path_buf(),
                source,
            })?;
        check_cancelled(&cancellation)?;
        temporary
            .persist(&destination)
            .map_err(|error| AppError::Io {
                operation: "publish version JSON",
                path: destination.clone(),
                source: error.error,
            })?;
        Ok(())
    })
    .await
    .map_err(|error| AppError::BlockingTask(error.to_string()))?
}

async fn read_bytes(path: &Path, operation: &'static str) -> Result<Vec<u8>, AppError> {
    tokio::fs::read(path).await.map_err(|source| AppError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    })
}

fn json_string(bytes: Vec<u8>, path: &Path) -> Result<String, AppError> {
    String::from_utf8(bytes).map_err(|source| AppError::JsonUtf8 {
        path: path.to_path_buf(),
        source,
    })
}

fn check_cancelled(cancellation: &CancellationToken) -> Result<(), AppError> {
    if cancellation.is_cancelled() {
        Err(AppError::Cancelled)
    } else {
        Ok(())
    }
}

async fn emit_phase(events: &Option<InstallEventSender>, phase: InstallPhase) {
    emit(events, InstallEvent::Phase(phase)).await;
}

async fn emit(events: &Option<InstallEventSender>, event: InstallEvent) {
    if let Some(events) = events {
        let _ = events.send(event).await;
    }
}

fn emit_blocking(events: &Option<InstallEventSender>, event: InstallEvent) {
    if let Some(events) = events {
        let _ = events.blocking_send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_download_cancellation_is_not_reported_as_a_failure() {
        let error = download_error(
            InstallPhase::PersistingVersion,
            rmcl_net::DownloadError::Cancelled,
            &CancellationToken::new(),
        );

        assert!(matches!(error, AppError::Cancelled));
    }
}
