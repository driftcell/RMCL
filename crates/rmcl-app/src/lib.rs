//! Headless application use cases and task orchestration.

mod accounts;
mod catalog;
mod content;
mod error;
mod game;
mod install;
mod instances;
mod java;
mod model;
mod skins;
mod task;
mod versions;

pub use accounts::{
    MICROSOFT_KEYRING_SERVICE, MicrosoftAccountService, MicrosoftAccountServiceError,
    ResolvedMicrosoftAccount,
};
pub use catalog::VersionCatalog;
pub use content::{
    ContentDownloadProgress, ContentInstallEvent, ContentInstallEventSender, ContentInstallOutcome,
    ContentInstallPhase, ContentInstallRequest, ContentInstaller, delete_instance_content,
    import_instance_content, scan_instance_content, set_instance_content_enabled,
};
pub use error::{AppError, InstallDownloadFailure};
pub use game::{
    GameLaunchEvent, GameLaunchEventSender, GameLaunchOutcome, GameLaunchRequest, GameLauncher,
};
pub use install::VanillaInstaller;
pub use instances::migrate_instance_to_isolated;
pub use java::{
    JavaEvent, JavaEventSender, JavaSelection, JavaSelectionOrigin, JavaService, JavaServiceConfig,
};
pub use model::{
    DownloadSummary, InstallDownloadProgress, InstallEvent, InstallEventSender, InstallOutcome,
    InstallPhase, VanillaInstallRequest, VanillaSources, VersionSource,
};
pub use rmcl_content::{
    ConflictPolicy, ContentKind, ContentLibrary, ContentSource, ContentWarning, LocalContent,
    ModrinthClient, ModrinthConfig, RemoteContentError, RemoteDependency, RemoteDependencyKind,
    RemoteFile, RemoteProject, RemoteSearchPage, RemoteSearchQuery, RemoteSort, RemoteVersion,
};
pub use rmcl_instance::{
    InstanceLayout, InstanceLoader, InstanceManifest, IsolationMode, LoaderKind, RepositoryPath,
};
pub use rmcl_minecraft::{VersionManifest, VersionSummary};
pub use rmcl_net::ProgressStage as DownloadStage;
pub use skins::{SkinCache, SkinError};
pub use task::{
    TaskEvent, TaskId, TaskKind, TaskManager, TaskPhase, TaskProgress, TaskSnapshot, TaskStatus,
    TaskSubject,
};
pub use versions::{InstalledVersion, delete_installed_version, discover_installed_versions};
