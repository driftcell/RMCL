use std::{collections::HashSet, io::Write, path::Path, sync::Arc};

use rmcl_archive::{ArchiveExtractor, ExtractionError, ExtractionReport};
use rmcl_domain::{DownloadRequest, Sha1Hash};
use rmcl_install::{
    InstallPlan, PlannedFileKind, plan_asset_objects_with_base, plan_vanilla, resolve_version,
};
use rmcl_instance::{
    InstanceLayout, InstanceManifest, IsolationMode, MinecraftLayout, RepositoryPath, VersionId,
};
use rmcl_minecraft::{AssetCatalog, GameVersion, VersionManifest, VersionSummary};
use rmcl_net::{
    DownloadConfig, DownloadExecutor, DownloadReport, MirrorHealth, ProgressStage, progress_channel,
};
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
    mirror: Option<Arc<MirrorHealth>>,
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
            mirror: None,
        })
    }

    /// Prefers the configured mirror for every generated request, falling back
    /// to the official URL through the download executor.
    pub fn with_mirror(mut self, mirror: Arc<MirrorHealth>) -> Self {
        self.mirror = Some(mirror);
        self
    }

    fn with_mirror_candidates(&self, mut request: DownloadRequest) -> DownloadRequest {
        if let Some(mirror) = &self.mirror {
            mirror.add_candidates(&mut request.sources);
        }
        request
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
        // A version JSON that already exists belongs to a user-visible instance.
        // Only a genuinely new installation is promoted to isolated by default;
        // reinstalling a legacy version must not silently move its game directory.
        let had_version_json =
            tokio::fs::try_exists(&version_json_path)
                .await
                .map_err(|source| AppError::Io {
                    operation: "check installed version JSON",
                    path: version_json_path.clone(),
                    source,
                })?;
        let version_json = self
            .resolve_version_json(&request, &version_json_path, &cancellation, &events)
            .await?;
        let version_json = json_string(version_json, &version_json_path)?;
        let child = GameVersion::from_json(&version_json)?;
        if child.id != request.version_id {
            return Err(AppError::VersionIdMismatch {
                expected: request.version_id.clone(),
                actual: child.id.clone(),
            });
        }
        let instance = resolve_instance_layout(&layout, &id, had_version_json, &request.source)?;
        let version = resolve_version(&layout, &id)?;

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

        if catalog.is_virtual == Some(true) || catalog.map_to_resources == Some(true) {
            emit_phase(&events, InstallPhase::MaterializingAssets).await;
            ensure_legacy_assets(&layout, &instance, &version, &catalog, &cancellation).await?;
        }

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
            instance,
            platform: request.platform,
            version,
            version_json_path,
            base_plan,
            asset_plan,
            downloads,
            natives,
        })
    }

    /// Loads an installed plan and verifies required base files without downloading.
    ///
    /// Asset objects are planned but not re-hashed; launch tolerates missing
    /// optional assets, while a missing client, library, native JAR, native
    /// directory, or asset index is reported as incomplete.
    pub async fn inspect_existing(
        &self,
        request: &VanillaInstallRequest,
        cancellation: CancellationToken,
    ) -> Result<InstallOutcome, AppError> {
        if !matches!(request.source, VersionSource::Existing) {
            return Err(AppError::InvalidRequest(
                "fast launch requires VersionSource::Existing".to_owned(),
            ));
        }
        check_cancelled(&cancellation)?;
        let root = std::path::absolute(&request.minecraft_root).map_err(|source| AppError::Io {
            operation: "resolve Minecraft root",
            path: request.minecraft_root.clone(),
            source,
        })?;
        let layout = MinecraftLayout::new(root);
        let id = VersionId::new(request.version_id.clone())?;
        let version_json_path = layout.version_json_path(&id);
        let bytes = read_bytes(&version_json_path, "read existing version JSON").await?;
        let json = json_string(bytes, &version_json_path)?;
        let child = GameVersion::from_json(&json)?;
        if child.id != request.version_id {
            return Err(AppError::VersionIdMismatch {
                expected: request.version_id.clone(),
                actual: child.id.clone(),
            });
        }
        let probe = layout.instance(id.clone(), IsolationMode::Shared);
        let manifest = probe.load_manifest()?;
        let isolation = manifest.map_or(IsolationMode::Shared, |manifest| manifest.isolation);
        let instance = layout.instance(id.clone(), isolation);
        let version = resolve_version(&layout, &id)?;

        let base_plan = plan_vanilla(&layout, &version, &request.platform)?;
        let mut missing = Vec::new();
        for file in &base_plan.files {
            if !tokio::fs::try_exists(&file.destination)
                .await
                .map_err(|source| AppError::Io {
                    operation: "check installed file",
                    path: file.destination.clone(),
                    source,
                })?
            {
                missing.push(file.destination.clone());
            }
        }
        let mut natives = Vec::new();
        if missing.is_empty() && !base_plan.extractions.is_empty() {
            let natives_dir = instance.natives_dir(&request.platform);
            if !natives_dir.is_dir() {
                // NOTE: A version installed by another launcher, or a partial
                // install left by an older RMCL, may lack RMCL's natives
                // directory while the native JARs are already present. Extract
                // them locally instead of failing launch and requiring a
                // network repair.
                let mut missing_sources = Vec::new();
                for extraction in &base_plan.extractions {
                    let exists =
                        tokio::fs::try_exists(&extraction.source)
                            .await
                            .map_err(|source| AppError::Io {
                                operation: "check native archive",
                                path: extraction.source.clone(),
                                source,
                            })?;
                    if !exists {
                        missing_sources.push(extraction.source.clone());
                    }
                }
                if missing_sources.is_empty() {
                    natives = merge_natives(
                        self.extractor.clone(),
                        base_plan.extractions.clone(),
                        cancellation.clone(),
                        None,
                    )
                    .await?;
                } else {
                    missing.extend(missing_sources);
                }
            }
        }
        if !missing.is_empty() {
            return Err(AppError::InstallationIncomplete {
                version: request.version_id.clone(),
                missing_count: missing.len(),
                first_missing: missing[0].display().to_string(),
            });
        }

        let asset_index = base_plan
            .files
            .iter()
            .find(|file| file.kind == PlannedFileKind::AssetIndex)
            .ok_or_else(|| AppError::InvalidRequest("base plan has no asset index".to_owned()))?;
        let catalog_json = read_bytes(&asset_index.destination, "read asset index").await?;
        let catalog_json = json_string(catalog_json, &asset_index.destination)?;
        let catalog = AssetCatalog::from_json(&catalog_json)?;
        let asset_plan =
            plan_asset_objects_with_base(&layout, &catalog, &self.sources.asset_object_base)?;
        ensure_legacy_assets(&layout, &instance, &version, &catalog, &cancellation).await?;
        check_cancelled(&cancellation)?;
        Ok(InstallOutcome {
            layout,
            instance,
            platform: request.platform.clone(),
            version,
            version_json_path,
            base_plan,
            asset_plan,
            downloads: DownloadSummary::default(),
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
                let request = self.with_mirror_candidates(request);
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
        let request = self.with_mirror_candidates(request);
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
        let requests = plan
            .files
            .iter()
            .filter(|file| !file.sources.is_empty())
            .map(|file| self.with_mirror_candidates(DownloadRequest::from(file)))
            .collect::<Vec<_>>();
        let total_files = requests.len();
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

fn resolve_instance_layout(
    layout: &MinecraftLayout,
    id: &VersionId,
    had_version_json: bool,
    source: &VersionSource,
) -> Result<InstanceLayout, AppError> {
    let probe = layout.instance(id.clone(), IsolationMode::Shared);
    let manifest = probe.load_manifest()?;
    let create_new =
        manifest.is_none() && !matches!(source, VersionSource::Existing) && !had_version_json;
    let isolation = if let Some(manifest) = &manifest {
        manifest.isolation
    } else if create_new {
        let instance = layout.instance(id.clone(), IsolationMode::Isolated);
        instance.write_manifest(&InstanceManifest::new(id, IsolationMode::Isolated))?;
        IsolationMode::Isolated
    } else {
        IsolationMode::Shared
    };
    let instance = layout.instance(id.clone(), isolation);
    if manifest.is_some() || create_new {
        instance.ensure_content_dirs()?;
    }
    Ok(instance)
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

/// Materializes legacy `virtual` and `map_to_resources` asset indexes.
///
/// Legacy indexes are not consumed directly by the game as content-addressed
/// objects. The virtual directory keeps assets under their original names, and
/// `map_to_resources` additionally needs a flat copy in the instance game
/// directory for pre-1.6 clients.
async fn ensure_legacy_assets(
    layout: &MinecraftLayout,
    instance: &InstanceLayout,
    version: &GameVersion,
    catalog: &AssetCatalog,
    cancellation: &CancellationToken,
) -> Result<(), AppError> {
    if catalog.is_virtual != Some(true) && catalog.map_to_resources != Some(true) {
        return Ok(());
    }
    let Some(asset_index) = version.asset_index.as_ref() else {
        return Err(AppError::InvalidRequest(format!(
            "version `{}` has legacy assets but no asset index",
            version.id
        )));
    };
    let virtual_root = layout.virtual_assets_dir(&asset_index.id)?;
    let resources_root =
        (catalog.map_to_resources == Some(true)).then(|| instance.game_dir().join("resources"));

    let mut assets = Vec::with_capacity(catalog.objects.len());
    let mut missing_count = 0;
    let mut first_missing = None;
    for (name, object) in &catalog.objects {
        check_cancelled(cancellation)?;
        let relative = RepositoryPath::new(name.clone())?;
        let hash: Sha1Hash = object.hash.parse()?;
        let source = layout.asset_object_path(hash);
        let source_size = tokio::fs::metadata(&source)
            .await
            .ok()
            .map(|metadata| metadata.len());
        if source_size != Some(object.size) {
            missing_count += 1;
            if first_missing.is_none() {
                first_missing = Some(source);
            }
            continue;
        }
        assets.push((source, relative, object.size));
    }
    if let Some(first_missing) = first_missing {
        return Err(AppError::InstallationIncomplete {
            version: version.id.clone(),
            missing_count,
            first_missing: first_missing.display().to_string(),
        });
    }

    for (source, relative, size) in assets {
        copy_asset_if_needed(
            &source,
            &relative.to_path(&virtual_root),
            size,
            cancellation,
        )
        .await?;
        if let Some(resources_root) = &resources_root {
            copy_asset_if_needed(
                &source,
                &relative.to_path(resources_root),
                size,
                cancellation,
            )
            .await?;
        }
    }
    Ok(())
}

async fn copy_asset_if_needed(
    source: &Path,
    target: &Path,
    size: u64,
    cancellation: &CancellationToken,
) -> Result<(), AppError> {
    check_cancelled(cancellation)?;
    match tokio::fs::symlink_metadata(target).await {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            tokio::fs::remove_file(target)
                .await
                .map_err(|source| AppError::Io {
                    operation: "remove legacy asset symlink",
                    path: target.to_path_buf(),
                    source,
                })?;
        }
        Ok(metadata) if metadata.is_file() && metadata.len() == size => return Ok(()),
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return Err(AppError::InvalidRequest(format!(
                "refusing to replace non-file legacy asset path `{}`",
                target.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(AppError::Io {
                operation: "inspect legacy asset target",
                path: target.to_path_buf(),
                source,
            });
        }
    }

    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|source| AppError::Io {
                operation: "create legacy asset directory",
                path: parent.to_path_buf(),
                source,
            })?;
    }
    if let Err(source_error) = tokio::fs::copy(source, target).await {
        let _ = tokio::fs::remove_file(target).await;
        return Err(AppError::Io {
            operation: "copy legacy asset",
            path: target.to_path_buf(),
            source: source_error,
        });
    }
    Ok(())
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
    use tempfile::TempDir;

    fn id(value: &str) -> VersionId {
        VersionId::new(value).unwrap()
    }

    #[test]
    fn single_download_cancellation_is_not_reported_as_a_failure() {
        let error = download_error(
            InstallPhase::PersistingVersion,
            rmcl_net::DownloadError::Cancelled,
            &CancellationToken::new(),
        );

        assert!(matches!(error, AppError::Cancelled));
    }

    #[test]
    fn a_new_install_creates_an_isolated_instance_manifest() {
        let temporary = TempDir::new().unwrap();
        let layout = MinecraftLayout::new(temporary.path());

        let instance =
            resolve_instance_layout(&layout, &id("1.21.1"), false, &VersionSource::Manifest)
                .unwrap();

        assert_eq!(instance.isolation(), IsolationMode::Isolated);
        assert!(
            instance
                .load_manifest()
                .unwrap()
                .is_some_and(|manifest| manifest.isolation == IsolationMode::Isolated)
        );
        assert!(instance.mods_dir().is_dir());
        assert!(instance.resourcepacks_dir().is_dir());
        assert!(instance.shaderpacks_dir().is_dir());
    }

    #[test]
    fn reinstalling_a_legacy_version_keeps_the_shared_game_directory() {
        let temporary = TempDir::new().unwrap();
        let layout = MinecraftLayout::new(temporary.path());

        let instance =
            resolve_instance_layout(&layout, &id("1.21.1"), true, &VersionSource::Manifest)
                .unwrap();

        assert_eq!(instance.isolation(), IsolationMode::Shared);
        assert!(instance.load_manifest().unwrap().is_none());
        assert_eq!(instance.game_dir(), temporary.path());
    }

    #[test]
    fn repair_never_creates_a_manifest() {
        let temporary = TempDir::new().unwrap();
        let layout = MinecraftLayout::new(temporary.path());

        let instance =
            resolve_instance_layout(&layout, &id("1.21.1"), false, &VersionSource::Existing)
                .unwrap();

        assert_eq!(instance.isolation(), IsolationMode::Shared);
        assert!(instance.load_manifest().unwrap().is_none());
    }

    #[tokio::test]
    async fn materializes_legacy_assets_into_virtual_and_resources_directories() {
        use sha1::{Digest, Sha1};

        let temporary = TempDir::new().unwrap();
        let layout = MinecraftLayout::new(temporary.path());
        let instance = layout.instance(id("b1.8.1"), IsolationMode::Isolated);
        let object = b"legacy asset".to_vec();
        let hash = hex::encode(Sha1::digest(&object));
        let source = layout.asset_object_path(hash.parse().unwrap());
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, &object).unwrap();

        let catalog = AssetCatalog::from_json(&format!(
            r#"{{"map_to_resources":true,"objects":{{"icons/icon.png":{{"hash":"{hash}","size":{}}}}}}}"#,
            object.len()
        ))
        .unwrap();
        let version = GameVersion::from_json(
            r#"{"id":"b1.8.1","assetIndex":{"id":"pre-1.6","url":"https://example.invalid/index.json"}}"#,
        )
        .unwrap();

        ensure_legacy_assets(
            &layout,
            &instance,
            &version,
            &catalog,
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        assert_eq!(
            std::fs::read(
                layout
                    .virtual_assets_dir("pre-1.6")
                    .unwrap()
                    .join("icons/icon.png")
            )
            .unwrap(),
            object
        );
        assert_eq!(
            std::fs::read(instance.game_dir().join("resources/icons/icon.png")).unwrap(),
            object
        );
    }

    #[test]
    fn an_existing_manifest_controls_isolation() {
        let temporary = TempDir::new().unwrap();
        let layout = MinecraftLayout::new(temporary.path());
        let isolated = layout.instance(id("1.21.1"), IsolationMode::Isolated);
        let mut manifest = InstanceManifest::new(&id("1.21.1"), IsolationMode::Isolated);
        manifest.name = "My Instance".to_owned();
        isolated.write_manifest(&manifest).unwrap();

        let instance =
            resolve_instance_layout(&layout, &id("1.21.1"), true, &VersionSource::Manifest)
                .unwrap();

        assert_eq!(instance.isolation(), IsolationMode::Isolated);
        assert_eq!(
            instance.load_manifest().unwrap().unwrap().name,
            "My Instance"
        );
    }
}
