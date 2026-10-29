use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use rmcl_content::{
    ConflictPolicy, ContentKind, ContentLibrary, InstalledMod, InstalledModIndex, LocalContent,
    ModDependency, ModMetadata, ModrinthClient, PackFormat, RemoteDependency, RemoteDependencyKind,
    RemoteFile, RemoteProject, RemoteVersion, ResourcePackOrder, delete_content, import_file,
    move_resource_pack, read_mod_metadata, replace_content_file, scan_library, set_content_enabled,
    version_satisfies,
};
use rmcl_domain::DownloadRequest;
use rmcl_instance::{InstanceLayout, MinecraftLayout, RepositoryPath, VersionId};
use rmcl_minecraft::GameVersion;
use rmcl_net::{DownloadConfig, DownloadExecutor, DownloadStatus, ProgressStage, progress_channel};
use sha1::{Digest, Sha1};
use tempfile::NamedTempFile;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{AppError, RepositoryWriteGuard, try_lock_repository};

const MAX_PREFLIGHT_JAR_BYTES: u64 = 16 * 1024 * 1024;

/// Reads the resource-pack format of an instance from its client JAR.
///
/// Loader instances point at the base client JAR through `version.json`'s
/// `jar` field. Missing metadata yields `None`; the caller should surface
/// compatibility as unknown rather than guessing.
pub fn resource_pack_format(instance: &InstanceLayout) -> Option<PackFormat> {
    let text = fs::read_to_string(instance.version_json_path()).ok()?;
    let version = GameVersion::from_json(&text).ok()?;
    let jar_id = version
        .jar
        .as_deref()
        .or(version.inherits_from.as_deref())
        .filter(|jar| !jar.trim().is_empty())
        .unwrap_or_else(|| instance.id().as_str());
    let jar_id = VersionId::new(jar_id).ok()?;
    rmcl_content::read_client_resource_pack_format(instance.client_jar_path_for(Some(&jar_id)))
        .ok()
        .flatten()
}

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
    let repository_guard = try_lock_repository(instance.repository().root())?;
    import_instance_content_with_guard(instance, kind, source, policy, &repository_guard).await
}

/// Imports content while the caller holds the repository write lock.
pub async fn import_instance_content_with_guard(
    instance: InstanceLayout,
    kind: ContentKind,
    source: PathBuf,
    policy: ConflictPolicy,
    _repository_guard: &RepositoryWriteGuard,
) -> Result<LocalContent, AppError> {
    tokio::task::spawn_blocking(move || import_file(&instance, kind, source, policy))
        .await
        .map_err(|error| AppError::BlockingTask(error.to_string()))?
        .map_err(AppError::Content)
}

/// Moves an enabled resource pack in the instance priority list.
pub async fn move_instance_resource_pack(
    instance: InstanceLayout,
    relative_path: RepositoryPath,
    order: ResourcePackOrder,
) -> Result<(), AppError> {
    tokio::task::spawn_blocking(move || move_resource_pack(&instance, &relative_path, order))
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
    let repository_guard = try_lock_repository(instance.repository().root())?;
    delete_instance_content_with_guard(instance, kind, relative_path, &repository_guard).await
}

/// Deletes content while the caller holds the repository write lock.
pub async fn delete_instance_content_with_guard(
    instance: InstanceLayout,
    kind: ContentKind,
    relative_path: RepositoryPath,
    _repository_guard: &RepositoryWriteGuard,
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
    let repository_guard = try_lock_repository(instance.repository().root())?;
    set_instance_content_enabled_with_guard(
        instance,
        kind,
        relative_path,
        enabled,
        &repository_guard,
    )
    .await
}

/// Toggles content while the caller holds the repository write lock.
pub async fn set_instance_content_enabled_with_guard(
    instance: InstanceLayout,
    kind: ContentKind,
    relative_path: RepositoryPath,
    enabled: bool,
    _repository_guard: &RepositoryWriteGuard,
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
    /// Existing relative path replaced only after a verified download.
    pub replace: Option<RepositoryPath>,
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
    /// Existing relative path for the primary file, replaced after verification.
    pub replace: Option<RepositoryPath>,
    /// Versions whose local mod ID/version already satisfy the requirement and
    /// must not be downloaded again as dependencies.
    pub skip_version_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentPreflightStatus {
    /// The exact file or the same mod ID and version is already present.
    AlreadyInstalled {
        enabled: bool,
    },
    /// The same mod ID is installed at a different version.
    InstalledDifferentVersion {
        version: Option<String>,
        enabled: bool,
    },
    WillInstall,
    /// An installed mod conflicts with a declared incompatible dependency.
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentPreflightItem {
    pub project_id: String,
    pub project_slug: String,
    pub title: String,
    pub version_id: String,
    pub version_number: String,
    pub file_name: String,
    pub status: ContentPreflightStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentInstallPreflight {
    pub main: ContentPreflightItem,
    pub dependencies: Vec<ContentPreflightItem>,
    pub incompatible: Vec<ContentPreflightItem>,
    pub optional_count: usize,
    pub warnings: Vec<String>,
    pub skip_version_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ContentBatchOutcome {
    pub items: Vec<ContentInstallOutcome>,
    pub downloaded_bytes: u64,
    pub skipped_files: usize,
}

/// Downloads a bounded mod archive once and caches its parsed metadata by SHA-1.
///
/// Preflight only reads metadata; the actual install still downloads through
/// `ContentInstaller` so its existing verification and atomic publish semantics
/// remain unchanged.
#[derive(Clone)]
pub struct ModMetadataCache {
    downloader: DownloadExecutor,
    directory: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct ModMetadataFetch {
    pub metadata: Option<Vec<ModMetadata>>,
    pub warning: Option<String>,
}

impl ModMetadataCache {
    pub fn new(config: DownloadConfig, directory: impl Into<PathBuf>) -> Result<Self, AppError> {
        Ok(Self {
            downloader: DownloadExecutor::new(config)?,
            directory: directory.into(),
        })
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Fetches and parses metadata for one remote archive.
    ///
    /// Files without a SHA-1 or larger than 16 MiB are skipped; the caller falls
    /// back to project-level matching.
    pub async fn fetch(
        &self,
        file: &RemoteFile,
        cancellation: &CancellationToken,
    ) -> Result<ModMetadataFetch, AppError> {
        if file.size.is_some_and(|size| size > MAX_PREFLIGHT_JAR_BYTES) {
            return Ok(ModMetadataFetch {
                metadata: None,
                warning: Some(format!(
                    "{} 超过 16 MiB，已跳过主文件元数据预检。",
                    file.filename
                )),
            });
        }
        let Some(sha1) = file.sha1 else {
            return Ok(ModMetadataFetch {
                metadata: None,
                warning: Some("文件没有 SHA-1，已跳过主文件元数据预检。".to_owned()),
            });
        };
        let path = self.directory.join(format!("{sha1}.jar"));
        if let Ok(metadata) = fs::metadata(&path)
            && metadata.len() > 0
        {
            return Ok(self.parse(&path).await);
        }

        let temporary = tempfile::tempdir().map_err(|source| AppError::Io {
            operation: "create content metadata temporary directory",
            path: self.directory.clone(),
            source,
        })?;
        let destination = temporary.path().join("content.jar");
        let request = DownloadRequest {
            sources: vec![file.url.clone()],
            destination: destination.clone(),
            sha1: Some(sha1),
            size: file.size,
        };
        self.downloader
            .download_file(&request, cancellation.clone(), None)
            .await
            .map_err(|error| AppError::ContentMetadata(error.to_string()))?;
        let parsed = self.parse(&destination).await;
        if let Err(source) = self.store(&destination, &path) {
            return Ok(ModMetadataFetch {
                metadata: parsed.metadata,
                warning: Some(match parsed.warning {
                    Some(warning) => format!("{warning}; 缓存元数据失败：{source}"),
                    None => format!("缓存主文件元数据失败：{source}"),
                }),
            });
        }
        Ok(parsed)
    }

    async fn parse(&self, path: &Path) -> ModMetadataFetch {
        let path = path.to_path_buf();
        match tokio::task::spawn_blocking(move || read_mod_metadata(&path).map_err(Box::new)).await
        {
            Ok(Ok(metadata)) => ModMetadataFetch {
                metadata: Some(metadata),
                warning: None,
            },
            Ok(Err(error)) => ModMetadataFetch {
                metadata: None,
                warning: Some(format!("无法读取主文件 Mod 元数据：{error}")),
            },
            Err(error) => ModMetadataFetch {
                metadata: None,
                warning: Some(format!("读取主文件 Mod 元数据失败：{error}")),
            },
        }
    }

    fn store(&self, source: &Path, path: &Path) -> Result<(), std::io::Error> {
        fs::create_dir_all(&self.directory)?;
        let mut temporary = NamedTempFile::new_in(&self.directory)?;
        let mut input = fs::File::open(source)?;
        std::io::copy(&mut input, temporary.as_file_mut())?;
        temporary.as_file().sync_all()?;
        match temporary.persist(path) {
            Ok(_) => Ok(()),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(error.error),
        }
    }
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
    let order = resolve_content_install_order(client, request, cancellation).await?;
    let mut requests = Vec::new();
    for version in order {
        if request.skip_version_ids.iter().any(|id| id == &version.id) {
            continue;
        }
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
            conflict: request.conflict,
            replace: if version.id == request.version.id {
                request.replace.clone()
            } else {
                None
            },
        });
    }
    Ok(requests)
}

/// Resolves the primary version and required dependencies into dependency-first
/// order without applying local skip rules.
pub async fn resolve_content_install_order(
    client: &ModrinthClient,
    request: &ContentGraphInstallRequest,
    cancellation: &CancellationToken,
) -> Result<Vec<RemoteVersion>, AppError> {
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
    Ok(order)
}

/// Builds a user-facing install preflight from the resolved dependency graph and
/// the local mod metadata index.
///
/// The preflight never downloads content. It reports whether a required
/// dependency is already present, will be installed, or conflicts with an
/// installed mod. Satisfied dependency versions are returned in
/// [`ContentInstallPreflight::skip_version_ids`] so the task does not create a
/// duplicate copy.
pub async fn preflight_content_install(
    client: &ModrinthClient,
    request: &ContentGraphInstallRequest,
    installed: &InstalledModIndex,
    metadata_cache: Option<&ModMetadataCache>,
    cancellation: &CancellationToken,
) -> Result<ContentInstallPreflight, AppError> {
    let order = resolve_content_install_order(client, request, cancellation).await?;
    let Some(main) = order.last().cloned() else {
        return Err(AppError::ContentDependency {
            message: "resolved dependency graph is empty".to_owned(),
        });
    };
    let mut projects = HashMap::new();
    let main_project = project_for_version(client, &main, &mut projects, cancellation).await?;
    let main_file = installable_file(&main, request.kind)?;
    let metadata_fetch = if request.kind == ContentKind::Mod {
        if let Some(cache) = metadata_cache {
            cache.fetch(&main_file, cancellation).await?
        } else {
            ModMetadataFetch::default()
        }
    } else {
        ModMetadataFetch::default()
    };
    let ModMetadataFetch {
        metadata,
        warning: metadata_warning,
    } = metadata_fetch;
    let main_metadata = metadata
        .as_deref()
        .and_then(|metadata| select_main_metadata(metadata, &main_project));
    let mut main_item = classify_preflight_item(
        request,
        &main,
        &main_project,
        installed,
        request.display_name.clone(),
    )?;
    if let Some(installed_mod) = main_metadata
        .and_then(|metadata| installed.find(&metadata.id))
        .or_else(|| find_installed_mod(installed, &main_project))
    {
        main_item.status = status_for_installed(installed_mod, &main.version_number);
    }

    let mut dependencies = Vec::new();
    let mut skip_version_ids = Vec::new();
    for version in &order[..order.len().saturating_sub(1)] {
        let project = project_for_version(client, version, &mut projects, cancellation).await?;
        let mut item = classify_preflight_item(
            request,
            version,
            &project,
            installed,
            if version.name.trim().is_empty() {
                project.title.clone()
            } else {
                version.name.clone()
            },
        )?;
        if let Some(metadata) = main_metadata
            && let Some(declared) = metadata_dependency_for(metadata, &project)
            && let Some(requirement) = declared.version_requirement.as_deref()
            && let Some(installed_mod) =
                find_installed_for_dependency(installed, &project, declared)
            && let Some(installed_version) = installed_mod.metadata.version.as_deref()
        {
            match version_satisfies(
                installed_version,
                requirement,
                metadata.loader.version_style(),
            ) {
                Some(true) => {
                    item.status = ContentPreflightStatus::AlreadyInstalled {
                        enabled: installed_mod.enabled,
                    };
                }
                Some(false) => {
                    item.status = ContentPreflightStatus::InstalledDifferentVersion {
                        version: Some(installed_version.to_owned()),
                        enabled: installed_mod.enabled,
                    };
                }
                None => {}
            }
        }
        if matches!(
            &item.status,
            ContentPreflightStatus::AlreadyInstalled { .. }
        ) {
            skip_version_ids.push(version.id.clone());
        }
        dependencies.push(item);
    }

    let mut incompatible = Vec::new();
    for dependency in main
        .dependencies
        .iter()
        .filter(|dependency| dependency.kind == RemoteDependencyKind::Incompatible)
    {
        let Some(project) =
            dependency_project(client, dependency, &mut projects, cancellation).await?
        else {
            continue;
        };
        if let Some(installed_mod) = find_installed_mod(installed, &project) {
            incompatible.push(ContentPreflightItem {
                project_id: project.id.clone(),
                project_slug: project.slug.clone(),
                title: project.title.clone(),
                version_id: dependency.version_id.clone().unwrap_or_default(),
                version_number: installed_mod.metadata.version.clone().unwrap_or_default(),
                file_name: installed_mod.file_name.clone(),
                status: ContentPreflightStatus::Conflict,
            });
        }
    }
    if let Some(metadata) = main_metadata {
        for conflict in &metadata.conflicts {
            let Some(installed_mod) = installed.find(&conflict.id) else {
                continue;
            };
            // Missing or unparsable versions stay conflicts because preflight
            // cannot prove that the installed pair is safe.
            if let (Some(requirement), Some(installed_version)) = (
                conflict.version_requirement.as_deref(),
                installed_mod.metadata.version.as_deref(),
            ) && !version_satisfies(
                installed_version,
                requirement,
                metadata.loader.version_style(),
            )
            .unwrap_or(true)
            {
                continue;
            }
            if incompatible
                .iter()
                .any(|item| item.file_name == installed_mod.file_name)
            {
                continue;
            }
            incompatible.push(ContentPreflightItem {
                project_id: String::new(),
                project_slug: conflict.id.clone(),
                title: conflict.id.clone(),
                version_id: String::new(),
                version_number: installed_mod.metadata.version.clone().unwrap_or_default(),
                file_name: installed_mod.file_name.clone(),
                status: ContentPreflightStatus::Conflict,
            });
        }
    }

    let mut warnings = installed.warnings.clone();
    if let Some(warning) = metadata_warning {
        warnings.push(warning);
    }
    for dependency in &dependencies {
        match &dependency.status {
            ContentPreflightStatus::AlreadyInstalled { enabled: false } => warnings.push(format!(
                "前置 {} 已安装但已禁用，请启用后再启动游戏。",
                dependency.title
            )),
            ContentPreflightStatus::InstalledDifferentVersion { version, .. } => {
                warnings.push(format!(
                    "已安装 {} 的另一版本{}，本次会安装所需的 {}。",
                    dependency.title,
                    version
                        .as_deref()
                        .map(|version| format!(" {version}"))
                        .unwrap_or_default(),
                    dependency.version_number
                ))
            }
            ContentPreflightStatus::AlreadyInstalled { enabled: true } => {}
            ContentPreflightStatus::WillInstall | ContentPreflightStatus::Conflict => {}
        }
    }
    if let Some(metadata) = main_metadata
        && let Some(installed_mod) = installed.find(&metadata.id)
        && !installed_mod.enabled
    {
        warnings.push(format!(
            "主 Mod {} 已安装但已禁用；本次安装会新增文件而不是启用现有文件。",
            metadata.id
        ));
    }
    if !incompatible.is_empty() {
        warnings.push(format!(
            "检测到 {} 个已安装的不兼容 Mod。",
            incompatible.len()
        ));
    }
    if request.kind == ContentKind::ShaderPack && !installed.has_shader_loader() {
        warnings.push(
            "当前实例未检测到 Iris、Oculus 或 OptiFine；光影文件可以安装，但不会生效。".to_owned(),
        );
    }
    if request.kind == ContentKind::Mod && request.loader.is_none() {
        warnings.push("当前实例没有 Mod 加载器，安装后 Mod 不会生效。".to_owned());
    }
    if main.version_type != "release" {
        warnings.push(format!(
            "这是一个 {} 版本，不是正式版；建议优先选择标记为“推荐”的正式版。",
            main.version_type
        ));
    }

    Ok(ContentInstallPreflight {
        main: main_item,
        dependencies,
        incompatible,
        optional_count: main
            .dependencies
            .iter()
            .filter(|dependency| dependency.kind == RemoteDependencyKind::Optional)
            .count(),
        warnings,
        skip_version_ids,
    })
}

fn classify_preflight_item(
    request: &ContentGraphInstallRequest,
    version: &RemoteVersion,
    project: &RemoteProject,
    installed: &InstalledModIndex,
    title: String,
) -> Result<ContentPreflightItem, AppError> {
    let file = installable_file(version, request.kind)?;
    let installed_mod = find_installed_mod(installed, project);
    let status = if content_already_installed(&request.instance, request.kind, &file) {
        ContentPreflightStatus::AlreadyInstalled {
            enabled: installed_mod.is_none_or(|installed| installed.enabled),
        }
    } else if let Some(installed_mod) = installed_mod {
        if same_mod_version(
            installed_mod.metadata.version.as_deref(),
            &version.version_number,
        ) {
            ContentPreflightStatus::AlreadyInstalled {
                enabled: installed_mod.enabled,
            }
        } else {
            ContentPreflightStatus::InstalledDifferentVersion {
                version: installed_mod.metadata.version.clone(),
                enabled: installed_mod.enabled,
            }
        }
    } else {
        ContentPreflightStatus::WillInstall
    };
    Ok(ContentPreflightItem {
        project_id: project.id.clone(),
        project_slug: project.slug.clone(),
        title,
        version_id: version.id.clone(),
        version_number: version.version_number.clone(),
        file_name: file.filename,
        status,
    })
}

fn find_installed_mod<'a>(
    installed: &'a InstalledModIndex,
    project: &RemoteProject,
) -> Option<&'a InstalledMod> {
    [project.slug.as_str(), project.title.as_str()]
        .into_iter()
        .filter_map(|candidate| installed.find(candidate))
        .next()
        .or_else(|| {
            let normalized = normalize_mod_id(&project.slug);
            installed.mods.iter().find(|installed| {
                normalize_mod_id(&installed.metadata.id) == normalized
                    || normalize_mod_id(installed.metadata.name.as_deref().unwrap_or_default())
                        == normalized
            })
        })
}

fn select_main_metadata<'a>(
    metadata: &'a [ModMetadata],
    project: &RemoteProject,
) -> Option<&'a ModMetadata> {
    metadata
        .iter()
        .find(|metadata| {
            normalize_mod_id(&metadata.id) == normalize_mod_id(&project.slug)
                || normalize_mod_id(&metadata.id) == normalize_mod_id(&project.title)
        })
        .or_else(|| metadata.first())
}

fn metadata_dependency_for<'a>(
    metadata: &'a ModMetadata,
    project: &RemoteProject,
) -> Option<&'a ModDependency> {
    metadata.dependencies.iter().find(|dependency| {
        normalize_mod_id(&dependency.id) == normalize_mod_id(&project.slug)
            || normalize_mod_id(&dependency.id) == normalize_mod_id(&project.title)
    })
}

fn find_installed_for_dependency<'a>(
    installed: &'a InstalledModIndex,
    project: &RemoteProject,
    dependency: &ModDependency,
) -> Option<&'a InstalledMod> {
    installed
        .find(&dependency.id)
        .or_else(|| find_installed_mod(installed, project))
}

fn status_for_installed(
    installed: &InstalledMod,
    required_version: &str,
) -> ContentPreflightStatus {
    if same_mod_version(installed.metadata.version.as_deref(), required_version) {
        ContentPreflightStatus::AlreadyInstalled {
            enabled: installed.enabled,
        }
    } else {
        ContentPreflightStatus::InstalledDifferentVersion {
            version: installed.metadata.version.clone(),
            enabled: installed.enabled,
        }
    }
}

fn normalize_mod_id(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn same_mod_version(installed: Option<&str>, required: &str) -> bool {
    installed.is_some_and(|installed| installed.eq_ignore_ascii_case(required))
}

async fn project_for_version(
    client: &ModrinthClient,
    version: &RemoteVersion,
    projects: &mut HashMap<String, RemoteProject>,
    cancellation: &CancellationToken,
) -> Result<RemoteProject, AppError> {
    if let Some(project) = projects.get(&version.project_id) {
        return Ok(project.clone());
    }
    let project = client.project(&version.project_id, cancellation).await?;
    projects.insert(version.project_id.clone(), project.clone());
    Ok(project)
}

async fn dependency_project(
    client: &ModrinthClient,
    dependency: &RemoteDependency,
    projects: &mut HashMap<String, RemoteProject>,
    cancellation: &CancellationToken,
) -> Result<Option<RemoteProject>, AppError> {
    if let Some(project_id) = dependency.project_id.as_deref() {
        if let Some(project) = projects.get(project_id) {
            return Ok(Some(project.clone()));
        }
        let project = client.project(project_id, cancellation).await?;
        projects.insert(project_id.to_owned(), project.clone());
        return Ok(Some(project));
    }
    if let Some(version_id) = dependency.version_id.as_deref() {
        let version = client.version(version_id, cancellation).await?;
        return project_for_version(client, &version, projects, cancellation)
            .await
            .map(Some);
    }
    Ok(None)
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
    let mut buffer = [0u8; 64 * 1024];
    loop {
        match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => hasher.update(&buffer[..read]),
            Err(_) => return false,
        }
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
        let replacement = request.replace.clone();
        let import_path = destination.clone();
        let content = tokio::task::spawn_blocking(move || match replacement {
            Some(relative_path) => {
                replace_content_file(&instance, kind, &import_path, &relative_path)
            }
            None => import_file(&instance, kind, &import_path, conflict),
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
