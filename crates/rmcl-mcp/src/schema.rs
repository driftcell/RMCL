use rmcl_app::{
    ContentInstallPhase, ContentInstallPreflight, ContentPreflightItem, ContentPreflightStatus,
    DownloadStage, InstallDownloadProgress, InstallPhase, InstalledVersion, InstanceCreatePhase,
    JavaRuntime, JavaRuntimeSource, LocalContent, RemoteProject, RemoteVersion,
    TaskKind as AppTaskKind, TaskPhase as AppTaskPhase, TaskProgress as AppTaskProgress,
    TaskSnapshot as AppTaskSnapshot, TaskStatus as AppTaskStatus, TaskSubject as AppTaskSubject,
};
use rmcl_launch::{GameExit, GameLog, LogStream};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::invalid_input;

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct LoaderSummary {
    pub kind: String,
    pub version: String,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct WindowSummary {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct InstanceSummary {
    pub id: String,
    pub name: String,
    pub minecraft_version: String,
    pub version_type: Option<String>,
    pub release_time: Option<String>,
    pub game_directory: String,
    pub isolated: bool,
    pub legacy: bool,
    pub loader: Option<LoaderSummary>,
    pub java: Option<String>,
    pub max_memory_mb: Option<u32>,
    pub window: Option<WindowSummary>,
    pub manifest_present: bool,
    pub manifest_error: Option<String>,
    pub client_jar_present: bool,
    pub revision: String,
}

pub fn instance_summary(version: &InstalledVersion) -> Result<InstanceSummary, rmcp::ErrorData> {
    Ok(InstanceSummary {
        id: version.id.as_str().to_owned(),
        name: version.display_name().to_owned(),
        minecraft_version: version.minecraft_version.clone(),
        version_type: version.version_type.clone(),
        release_time: version.release_time.clone(),
        game_directory: version.game_dir.display().to_string(),
        isolated: version.is_isolated(),
        legacy: version.is_legacy(),
        loader: version.loader.as_ref().map(|loader| LoaderSummary {
            kind: loader.kind.as_str().to_owned(),
            version: loader.version.clone(),
        }),
        java: version.java.as_ref().map(|path| path.display().to_string()),
        max_memory_mb: version.max_memory_mb,
        window: version
            .window_width
            .zip(version.window_height)
            .map(|(width, height)| WindowSummary { width, height }),
        manifest_present: version.manifest_present,
        manifest_error: version.manifest_error.clone(),
        client_jar_present: version.client_jar_present,
        revision: instance_revision(version)?,
    })
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct InstanceList {
    pub items: Vec<InstanceSummary>,
}

pub fn instance_revision(version: &InstalledVersion) -> Result<String, rmcp::ErrorData> {
    let json_path = version
        .version_dir
        .join(format!("{}.json", version.id.as_str()));
    let manifest_path = version.version_dir.join("rmcl.instance.toml");
    let mut hasher = Sha256::new();
    hasher.update(std::fs::read(json_path).map_err(crate::error::internal_error)?);
    match std::fs::read(manifest_path) {
        Ok(manifest) => {
            hasher.update([0]);
            hasher.update(manifest);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(crate::error::internal_error(error)),
    }
    Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct ContentEntry {
    pub kind: String,
    pub name: String,
    pub relative_path: String,
    pub size: Option<u64>,
    pub enabled: bool,
    pub is_directory: bool,
    pub can_toggle: bool,
}

pub fn content_entry(item: &LocalContent) -> ContentEntry {
    ContentEntry {
        kind: item.kind.modrinth_project_type().to_owned(),
        name: item.display_name().to_owned(),
        relative_path: item.relative_path.as_str().to_owned(),
        size: item.size,
        enabled: item.enabled,
        is_directory: item.is_directory,
        can_toggle: item.can_toggle(),
    }
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct ContentList {
    pub instance_id: String,
    pub items: Vec<ContentEntry>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct ContentWrite {
    pub content: ContentEntry,
    pub revision: String,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct ContentDelete {
    pub deleted: bool,
    pub revision: String,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JavaRuntimeSourceDto {
    Managed,
    JavaHome,
    System,
    Custom,
}

impl From<JavaRuntimeSource> for JavaRuntimeSourceDto {
    fn from(source: JavaRuntimeSource) -> Self {
        match source {
            JavaRuntimeSource::Managed => Self::Managed,
            JavaRuntimeSource::JavaHome => Self::JavaHome,
            JavaRuntimeSource::System => Self::System,
            JavaRuntimeSource::Custom => Self::Custom,
        }
    }
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct JavaRuntimeInfo {
    pub executable: String,
    pub version: String,
    pub major_version: u32,
    pub operating_system: String,
    pub architecture: String,
    pub vendor: Option<String>,
    pub source: JavaRuntimeSourceDto,
}

pub fn java_runtime(runtime: &JavaRuntime) -> JavaRuntimeInfo {
    JavaRuntimeInfo {
        executable: runtime.executable.display().to_string(),
        version: runtime.version.clone(),
        major_version: runtime.major_version,
        operating_system: runtime.operating_system.as_str().to_owned(),
        architecture: runtime.architecture.as_str().to_owned(),
        vendor: runtime.vendor.clone(),
        source: runtime.source.into(),
    }
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct JavaFailure {
    pub executable: String,
    pub error: String,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct JavaList {
    pub items: Vec<JavaRuntimeInfo>,
    pub failures: Vec<JavaFailure>,
}

#[derive(Debug, Clone, Copy, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    VanillaInstall,
    InstanceCreate,
    ContentInstall,
    GameLaunch,
    JavaInstall,
}

impl From<AppTaskKind> for TaskKind {
    fn from(kind: AppTaskKind) -> Self {
        match kind {
            AppTaskKind::VanillaInstall => Self::VanillaInstall,
            AppTaskKind::InstanceCreate => Self::InstanceCreate,
            AppTaskKind::ContentInstall => Self::ContentInstall,
            AppTaskKind::GameLaunch => Self::GameLaunch,
            AppTaskKind::JavaInstall => Self::JavaInstall,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Running,
    Cancelling,
    Completed,
    Cancelled,
    Failed,
    Exited,
}

impl From<AppTaskStatus> for TaskStatus {
    fn from(status: AppTaskStatus) -> Self {
        match status {
            AppTaskStatus::Running => Self::Running,
            AppTaskStatus::Cancelling => Self::Cancelling,
            AppTaskStatus::Completed => Self::Completed,
            AppTaskStatus::Cancelled => Self::Cancelled,
            AppTaskStatus::Failed => Self::Failed,
            AppTaskStatus::Exited => Self::Exited,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InstallPhaseDto {
    ResolvingVersion,
    ReadingVersion,
    PersistingVersion,
    PlanningBase,
    DownloadingBase,
    ReadingAssetIndex,
    PlanningAssets,
    DownloadingAssets,
    MaterializingAssets,
    DownloadingLoader,
    MergingNatives,
    Completed,
}

impl From<InstallPhase> for InstallPhaseDto {
    fn from(phase: InstallPhase) -> Self {
        match phase {
            InstallPhase::ResolvingVersion => Self::ResolvingVersion,
            InstallPhase::ReadingVersion => Self::ReadingVersion,
            InstallPhase::PersistingVersion => Self::PersistingVersion,
            InstallPhase::PlanningBase => Self::PlanningBase,
            InstallPhase::DownloadingBase => Self::DownloadingBase,
            InstallPhase::ReadingAssetIndex => Self::ReadingAssetIndex,
            InstallPhase::PlanningAssets => Self::PlanningAssets,
            InstallPhase::DownloadingAssets => Self::DownloadingAssets,
            InstallPhase::MaterializingAssets => Self::MaterializingAssets,
            InstallPhase::DownloadingLoader => Self::DownloadingLoader,
            InstallPhase::MergingNatives => Self::MergingNatives,
            InstallPhase::Completed => Self::Completed,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InstanceCreatePhaseDto {
    Preparing,
    InstallingBase,
    FetchingLoader,
    WritingInstance,
    CopyingContent,
    InstallingLoader,
    Completed,
}

impl From<InstanceCreatePhase> for InstanceCreatePhaseDto {
    fn from(phase: InstanceCreatePhase) -> Self {
        match phase {
            InstanceCreatePhase::Preparing => Self::Preparing,
            InstanceCreatePhase::InstallingBase => Self::InstallingBase,
            InstanceCreatePhase::FetchingLoader => Self::FetchingLoader,
            InstanceCreatePhase::WritingInstance => Self::WritingInstance,
            InstanceCreatePhase::CopyingContent => Self::CopyingContent,
            InstanceCreatePhase::InstallingLoader => Self::InstallingLoader,
            InstanceCreatePhase::Completed => Self::Completed,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContentInstallPhaseDto {
    Preparing,
    Downloading,
    Installing,
    Completed,
}

impl From<ContentInstallPhase> for ContentInstallPhaseDto {
    fn from(phase: ContentInstallPhase) -> Self {
        match phase {
            ContentInstallPhase::Preparing => Self::Preparing,
            ContentInstallPhase::Downloading => Self::Downloading,
            ContentInstallPhase::Installing => Self::Installing,
            ContentInstallPhase::Completed => Self::Completed,
        }
    }
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub enum TaskPhase {
    Install(InstallPhaseDto),
    InstanceCreate(InstanceCreatePhaseDto),
    Content(ContentInstallPhaseDto),
    Inspecting,
    CheckingJava,
    InstallingJava,
    PreparingLaunch,
    StartingGame,
    RunningGame,
}

impl From<AppTaskPhase> for TaskPhase {
    fn from(phase: AppTaskPhase) -> Self {
        match phase {
            AppTaskPhase::Install(phase) => Self::Install(phase.into()),
            AppTaskPhase::InstanceCreate(phase) => Self::InstanceCreate(phase.into()),
            AppTaskPhase::Content(phase) => Self::Content(phase.into()),
            AppTaskPhase::Inspecting => Self::Inspecting,
            AppTaskPhase::CheckingJava => Self::CheckingJava,
            AppTaskPhase::InstallingJava => Self::InstallingJava,
            AppTaskPhase::PreparingLaunch => Self::PreparingLaunch,
            AppTaskPhase::StartingGame => Self::StartingGame,
            AppTaskPhase::RunningGame => Self::RunningGame,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProgressStage {
    Checking,
    Downloading,
    Verifying,
    Completed,
    Skipped,
    Failed,
    Cancelled,
}

impl From<DownloadStage> for ProgressStage {
    fn from(stage: DownloadStage) -> Self {
        match stage {
            DownloadStage::Checking => Self::Checking,
            DownloadStage::Downloading => Self::Downloading,
            DownloadStage::Verifying => Self::Verifying,
            DownloadStage::Completed => Self::Completed,
            DownloadStage::Skipped => Self::Skipped,
            DownloadStage::Failed => Self::Failed,
            DownloadStage::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskProgress {
    Download {
        phase: InstallPhaseDto,
        stage: ProgressStage,
        current: String,
        completed_files: usize,
        total_files: usize,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        detail: Option<String>,
    },
    Content {
        phase: ContentInstallPhaseDto,
        stage: ProgressStage,
        current: String,
        completed_files: usize,
        total_files: usize,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        detail: Option<String>,
    },
    Processors {
        current: String,
        completed: usize,
        total: usize,
    },
    Natives {
        current: String,
        completed_archives: usize,
        total_archives: usize,
    },
}

impl From<&InstallDownloadProgress> for TaskProgress {
    fn from(progress: &InstallDownloadProgress) -> Self {
        Self::Download {
            phase: progress.phase.into(),
            stage: progress.stage.into(),
            current: progress.current.display().to_string(),
            completed_files: progress.completed_files,
            total_files: progress.total_files,
            downloaded_bytes: progress.downloaded_bytes,
            total_bytes: progress.total_bytes,
            detail: progress.detail.clone(),
        }
    }
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskSubject {
    Version {
        id: String,
    },
    Content {
        instance_id: String,
        kind: String,
        name: String,
    },
    Instance {
        id: String,
        name: String,
    },
    Java {
        major_version: Option<u32>,
        component: Option<String>,
    },
}

impl From<&AppTaskSubject> for TaskSubject {
    fn from(subject: &AppTaskSubject) -> Self {
        match subject {
            AppTaskSubject::Version { id } => Self::Version { id: id.clone() },
            AppTaskSubject::Content {
                instance_id,
                kind,
                name,
            } => Self::Content {
                instance_id: instance_id.clone(),
                kind: kind.modrinth_project_type().to_owned(),
                name: name.clone(),
            },
            AppTaskSubject::Instance { id, name } => Self::Instance {
                id: id.clone(),
                name: name.clone(),
            },
            AppTaskSubject::Java {
                major_version,
                component,
            } => Self::Java {
                major_version: *major_version,
                component: component.clone(),
            },
        }
    }
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct GameExitInfo {
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub success: bool,
}

impl From<&GameExit> for GameExitInfo {
    fn from(exit: &GameExit) -> Self {
        Self {
            code: exit.code,
            signal: exit.signal,
            success: exit.success,
        }
    }
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct TaskSnapshot {
    pub id: u64,
    pub kind: TaskKind,
    pub subject: Option<TaskSubject>,
    pub status: TaskStatus,
    pub phase: Option<TaskPhase>,
    pub progress: Option<TaskProgress>,
    pub error: Option<String>,
    pub exit: Option<GameExitInfo>,
    pub stop_requested: bool,
    pub log_lines: u64,
}

pub fn task_snapshot(snapshot: &AppTaskSnapshot) -> TaskSnapshot {
    TaskSnapshot {
        id: snapshot.id.get(),
        kind: snapshot.kind.into(),
        subject: snapshot.subject.as_ref().map(TaskSubject::from),
        status: snapshot.status.into(),
        phase: snapshot.phase.map(TaskPhase::from),
        progress: snapshot.progress.as_ref().map(|progress| match progress {
            AppTaskProgress::Download(progress) => progress.into(),
            AppTaskProgress::Content(progress) => TaskProgress::Content {
                phase: progress.phase.into(),
                stage: progress.stage.into(),
                current: progress.current.display().to_string(),
                completed_files: progress.completed_files,
                total_files: progress.total_files,
                downloaded_bytes: progress.downloaded_bytes,
                total_bytes: progress.total_bytes,
                detail: progress.detail.clone(),
            },
            AppTaskProgress::Processors {
                current,
                completed,
                total,
            } => TaskProgress::Processors {
                current: current.clone(),
                completed: *completed,
                total: *total,
            },
            AppTaskProgress::Natives {
                current,
                completed_archives,
                total_archives,
            } => TaskProgress::Natives {
                current: current.display().to_string(),
                completed_archives: *completed_archives,
                total_archives: *total_archives,
            },
        }),
        error: snapshot.error.clone(),
        exit: snapshot.exit.as_ref().map(GameExitInfo::from),
        stop_requested: snapshot.stop_requested,
        log_lines: snapshot.log_lines,
    }
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct TaskList {
    pub items: Vec<TaskSnapshot>,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct TaskStarted {
    pub task_id: u64,
    pub status: TaskStatus,
    pub idempotent_replay: bool,
}

impl TaskStarted {
    pub fn new(task_id: u64, idempotent_replay: bool) -> Self {
        Self {
            task_id,
            status: TaskStatus::Running,
            idempotent_replay,
        }
    }
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct TaskSubscription {
    pub subscription_id: u64,
    pub task_id: Option<u64>,
    pub include_logs: bool,
    pub items: Vec<TaskSnapshot>,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct TaskSubscriptionEnded {
    pub subscription_id: u64,
    pub unsubscribed: bool,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GameLogStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct GameLogEntry {
    pub stream: GameLogStream,
    pub text: String,
}

pub fn game_log(log: &GameLog) -> GameLogEntry {
    GameLogEntry {
        stream: match log.stream {
            LogStream::Stdout => GameLogStream::Stdout,
            LogStream::Stderr => GameLogStream::Stderr,
        },
        text: log.text.clone(),
    }
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskNotification {
    Snapshot {
        subscription_id: u64,
        task: TaskSnapshot,
    },
    Log {
        subscription_id: u64,
        task_id: u64,
        line: u64,
        log: GameLogEntry,
    },
    Dismissed {
        subscription_id: u64,
        task_id: u64,
    },
    Lagged {
        subscription_id: u64,
        missed: u64,
    },
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct LogPage {
    pub items: Vec<GameLogEntry>,
    pub next_cursor: Option<usize>,
}

pub fn log_page(
    logs: &[GameLog],
    cursor: Option<usize>,
    limit: Option<usize>,
) -> Result<LogPage, rmcp::ErrorData> {
    let limit = limit.unwrap_or(200).clamp(1, 1000);
    let start = cursor.unwrap_or(0);
    if start > logs.len() {
        return Err(invalid_input("log cursor is past the end of the log"));
    }
    let end = start.saturating_add(limit).min(logs.len());
    let next_cursor = (end < logs.len()).then_some(end);
    Ok(LogPage {
        items: logs[start..end].iter().map(game_log).collect(),
        next_cursor,
    })
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct PlatformSummary {
    pub os: String,
    pub os_version: String,
    pub architecture: String,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct PathSummary {
    pub config_file: String,
    pub minecraft_root: String,
    pub java_store: String,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct Capabilities {
    pub name: String,
    pub version: String,
    pub access: String,
    pub transports: Vec<String>,
    pub tools: Vec<String>,
    pub platform: PlatformSummary,
    pub paths: PathSummary,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct CreateLoaderPlan {
    pub kind: String,
    pub version: String,
    pub installer_version: String,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct CreatePlatformPlan {
    pub os: String,
    pub architecture: String,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct CreateInstancePlan {
    pub dry_run: bool,
    pub instance_id: String,
    pub name: String,
    pub base_version_id: String,
    pub loader: Option<CreateLoaderPlan>,
    pub copy_content_from: Option<String>,
    pub platform: CreatePlatformPlan,
    pub checks: Vec<PreflightCheck>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
#[serde(untagged)]
pub enum CreateInstanceResult {
    Plan(Box<CreateInstancePlan>),
    Task(TaskStarted),
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct RemoteProjectSummary {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub author: String,
    pub downloads: u64,
    pub follows: u64,
    pub categories: Vec<String>,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub project_type: String,
    pub updated: Option<String>,
    pub page_url: String,
}

impl From<&RemoteProject> for RemoteProjectSummary {
    fn from(project: &RemoteProject) -> Self {
        Self {
            id: project.id.clone(),
            slug: project.slug.clone(),
            title: project.title.clone(),
            description: project.description.clone(),
            author: project.author.clone(),
            downloads: project.downloads,
            follows: project.follows,
            categories: project.categories.clone(),
            game_versions: project.game_versions.clone(),
            loaders: project.loaders.clone(),
            project_type: project.project_type.clone(),
            updated: project.updated.clone(),
            page_url: project.page_url().to_string(),
        }
    }
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct RemoteSearchPage {
    pub total_hits: u64,
    pub offset: usize,
    pub limit: usize,
    pub items: Vec<RemoteProjectSummary>,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct RemoteFileSummary {
    pub filename: String,
    pub url: String,
    pub size: Option<u64>,
    pub sha1: Option<String>,
    pub primary: bool,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct RemoteDependencySummary {
    pub version_id: Option<String>,
    pub project_id: Option<String>,
    pub file_name: Option<String>,
    pub kind: String,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct RemoteVersionSummary {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
    pub version_type: String,
    pub published: Option<String>,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub files: Vec<RemoteFileSummary>,
    pub dependencies: Vec<RemoteDependencySummary>,
}

impl From<&RemoteVersion> for RemoteVersionSummary {
    fn from(version: &RemoteVersion) -> Self {
        Self {
            id: version.id.clone(),
            project_id: version.project_id.clone(),
            name: version.name.clone(),
            version_number: version.version_number.clone(),
            version_type: version.version_type.clone(),
            published: version.published.clone(),
            game_versions: version.game_versions.clone(),
            loaders: version.loaders.clone(),
            files: version
                .files
                .iter()
                .map(|file| RemoteFileSummary {
                    filename: file.filename.clone(),
                    url: file.url.to_string(),
                    size: file.size,
                    sha1: file.sha1.map(|sha1| sha1.to_string()),
                    primary: file.primary,
                })
                .collect(),
            dependencies: version
                .dependencies
                .iter()
                .map(|dependency| RemoteDependencySummary {
                    version_id: dependency.version_id.clone(),
                    project_id: dependency.project_id.clone(),
                    file_name: dependency.file_name.clone(),
                    kind: format!("{:?}", dependency.kind).to_lowercase(),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct RemoteVersionList {
    pub items: Vec<RemoteVersionSummary>,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ContentPreflightStatusDto {
    AlreadyInstalled {
        enabled: bool,
    },
    InstalledDifferentVersion {
        version: Option<String>,
        enabled: bool,
    },
    WillInstall,
    Conflict,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct ContentPreflightItemDto {
    pub project_id: String,
    pub project_slug: String,
    pub title: String,
    pub version_id: String,
    pub version_number: String,
    pub file_name: String,
    pub status: ContentPreflightStatusDto,
}

impl From<&ContentPreflightItem> for ContentPreflightItemDto {
    fn from(item: &ContentPreflightItem) -> Self {
        let status = match &item.status {
            ContentPreflightStatus::AlreadyInstalled { enabled } => {
                ContentPreflightStatusDto::AlreadyInstalled { enabled: *enabled }
            }
            ContentPreflightStatus::InstalledDifferentVersion { version, enabled } => {
                ContentPreflightStatusDto::InstalledDifferentVersion {
                    version: version.clone(),
                    enabled: *enabled,
                }
            }
            ContentPreflightStatus::WillInstall => ContentPreflightStatusDto::WillInstall,
            ContentPreflightStatus::Conflict => ContentPreflightStatusDto::Conflict,
        };
        Self {
            project_id: item.project_id.clone(),
            project_slug: item.project_slug.clone(),
            title: item.title.clone(),
            version_id: item.version_id.clone(),
            version_number: item.version_number.clone(),
            file_name: item.file_name.clone(),
            status,
        }
    }
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct ContentInstallPreflightDto {
    pub dry_run: bool,
    pub main: ContentPreflightItemDto,
    pub dependencies: Vec<ContentPreflightItemDto>,
    pub incompatible: Vec<ContentPreflightItemDto>,
    pub optional_count: usize,
    pub warnings: Vec<String>,
    pub skip_version_ids: Vec<String>,
}

impl From<&ContentInstallPreflight> for ContentInstallPreflightDto {
    fn from(preflight: &ContentInstallPreflight) -> Self {
        Self {
            dry_run: true,
            main: (&preflight.main).into(),
            dependencies: preflight.dependencies.iter().map(Into::into).collect(),
            incompatible: preflight.incompatible.iter().map(Into::into).collect(),
            optional_count: preflight.optional_count,
            warnings: preflight.warnings.clone(),
            skip_version_ids: preflight.skip_version_ids.clone(),
        }
    }
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
#[serde(untagged)]
pub enum ContentInstallResult {
    Preflight(Box<ContentInstallPreflightDto>),
    Task(TaskStarted),
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct MinecraftVersionSummary {
    pub id: String,
    pub version_type: Option<String>,
    pub url: String,
    pub sha1: Option<String>,
    pub time: Option<String>,
    pub release_time: Option<String>,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct MinecraftVersionList {
    pub latest_release: Option<String>,
    pub latest_snapshot: Option<String>,
    pub items: Vec<MinecraftVersionSummary>,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct LoaderVersionInfo {
    pub version: String,
    pub installer_version: String,
    pub stable: bool,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct LoaderVersionList {
    pub kind: String,
    pub game_version: String,
    pub items: Vec<LoaderVersionInfo>,
}

#[derive(Debug, Clone, Copy, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PreflightCheckStatus {
    Passed,
    Warning,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct PreflightCheck {
    pub name: String,
    pub status: PreflightCheckStatus,
    pub message: String,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct InstanceDeletePlan {
    pub dry_run: bool,
    pub instance_id: String,
    pub version_directory: String,
    pub deletes_isolated_content: bool,
    pub preserves_shared_files: bool,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct InstanceDelete {
    pub deleted: bool,
    pub instance_id: String,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
#[serde(untagged)]
pub enum InstanceDeleteResult {
    Plan(InstanceDeletePlan),
    Deleted(InstanceDelete),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccountKind {
    Offline,
    Microsoft,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct AccountSummary {
    pub username: String,
    pub uuid: String,
    pub kind: AccountKind,
    pub skin_url: Option<String>,
    pub selected: bool,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct AccountList {
    pub items: Vec<AccountSummary>,
    pub selected_account: Option<String>,
    pub selected_account_kind: AccountKind,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct AccountWrite {
    pub account: AccountSummary,
}

#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct AccountDelete {
    pub deleted: bool,
    pub username: String,
    pub kind: AccountKind,
}

#[cfg(test)]
mod tests {
    use rmcl_app::{TaskId, TaskProgress as AppTaskProgress};
    use serde_json::json;

    use super::*;

    #[test]
    fn instance_revision_covers_version_json_and_manifest() {
        let temporary = tempfile::TempDir::new().unwrap();
        let root = temporary.path().join("minecraft");
        let version_dir = root.join("versions/example");
        std::fs::create_dir_all(&version_dir).unwrap();
        let json_path = version_dir.join("example.json");
        let manifest_path = version_dir.join("rmcl.instance.toml");
        std::fs::write(&json_path, br#"{"id":"example","type":"release"}"#).unwrap();
        let instance = rmcl_app::discover_installed_versions(&root)
            .unwrap()
            .remove(0);

        let without_manifest = instance_revision(&instance).unwrap();
        std::fs::write(&manifest_path, b"name = \"Example\"\n").unwrap();
        let with_manifest = instance_revision(&instance).unwrap();
        assert_ne!(without_manifest, with_manifest);

        std::fs::write(&manifest_path, b"name = \"Renamed\"\n").unwrap();
        let renamed = instance_revision(&instance).unwrap();
        assert_ne!(with_manifest, renamed);

        std::fs::write(
            &json_path,
            br#"{"id":"example","type":"release","releaseTime":"2026-01-01T00:00:00Z"}"#,
        )
        .unwrap();
        assert_ne!(renamed, instance_revision(&instance).unwrap());
    }

    #[test]
    fn log_pages_are_bounded_and_report_the_next_cursor() {
        let logs = (0..3)
            .map(|index| GameLog {
                stream: LogStream::Stdout,
                text: format!("line {index}"),
            })
            .collect::<Vec<_>>();

        let first = log_page(&logs, None, Some(2)).unwrap();
        assert_eq!(first.items.len(), 2);
        assert_eq!(first.next_cursor, Some(2));
        let second = log_page(&logs, Some(2), Some(2)).unwrap();
        assert_eq!(second.items.len(), 1);
        assert_eq!(second.next_cursor, None);
        assert!(log_page(&logs, Some(4), None).is_err());
    }

    #[test]
    fn task_snapshots_use_stable_typed_json() {
        let snapshot = AppTaskSnapshot {
            id: TaskId::from_raw(42),
            kind: AppTaskKind::ContentInstall,
            subject: Some(AppTaskSubject::Content {
                instance_id: "example".to_owned(),
                kind: rmcl_app::ContentKind::Mod,
                name: "Example".to_owned(),
            }),
            status: AppTaskStatus::Running,
            phase: Some(AppTaskPhase::Content(ContentInstallPhase::Downloading)),
            progress: Some(AppTaskProgress::Content(
                rmcl_app::ContentDownloadProgress {
                    phase: ContentInstallPhase::Downloading,
                    stage: DownloadStage::Downloading,
                    current: "mods/example.jar".into(),
                    completed_files: 1,
                    total_files: 2,
                    downloaded_bytes: 50,
                    total_bytes: Some(100),
                    detail: None,
                },
            )),
            error: None,
            exit: None,
            stop_requested: false,
            log_lines: 3,
        };

        let value = serde_json::to_value(task_snapshot(&snapshot)).unwrap();
        assert_eq!(value["kind"], "content_install");
        assert_eq!(value["subject"]["type"], "content");
        assert_eq!(value["subject"]["kind"], "mod");
        assert_eq!(value["phase"]["kind"], "content");
        assert_eq!(value["phase"]["name"], "downloading");
        assert_eq!(value["progress"]["type"], "content");
        assert_eq!(value["progress"]["stage"], "downloading");
        assert_eq!(value["progress"]["current"], "mods/example.jar");
        assert_eq!(value["progress"]["downloaded_bytes"], 50);
    }

    #[test]
    fn install_progress_and_exit_have_typed_json() {
        let snapshot = AppTaskSnapshot {
            id: TaskId::from_raw(7),
            kind: AppTaskKind::VanillaInstall,
            subject: Some(AppTaskSubject::Version {
                id: "1.21.1".to_owned(),
            }),
            status: AppTaskStatus::Exited,
            phase: Some(AppTaskPhase::Install(InstallPhase::DownloadingBase)),
            progress: Some(AppTaskProgress::Download(InstallDownloadProgress {
                phase: InstallPhase::DownloadingBase,
                stage: DownloadStage::Verifying,
                current: "libraries/example.jar".into(),
                completed_files: 3,
                total_files: 8,
                downloaded_bytes: 120,
                total_bytes: Some(200),
                detail: Some("checking".to_owned()),
            })),
            error: None,
            exit: Some(GameExit {
                code: Some(0),
                signal: None,
                success: true,
            }),
            stop_requested: false,
            log_lines: 0,
        };

        let value = serde_json::to_value(task_snapshot(&snapshot)).unwrap();
        assert_eq!(
            value["phase"],
            json!({"kind":"install","name":"downloading_base"})
        );
        assert_eq!(value["progress"]["type"], "download");
        assert_eq!(value["progress"]["stage"], "verifying");
        assert_eq!(
            value["exit"],
            json!({"code":0,"signal":null,"success":true})
        );
    }
}
