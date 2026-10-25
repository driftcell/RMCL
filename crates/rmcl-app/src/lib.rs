//! Headless application use cases and task orchestration.

mod catalog;
mod error;
mod game;
mod install;
mod java;
mod model;
mod task;
mod versions;

pub use catalog::VersionCatalog;
pub use error::{AppError, InstallDownloadFailure};
pub use game::{
    GameLaunchEvent, GameLaunchEventSender, GameLaunchOutcome, GameLaunchRequest, GameLauncher,
};
pub use install::VanillaInstaller;
pub use java::{
    JavaEvent, JavaEventSender, JavaSelection, JavaSelectionOrigin, JavaService, JavaServiceConfig,
};
pub use model::{
    DownloadSummary, InstallDownloadProgress, InstallEvent, InstallEventSender, InstallOutcome,
    InstallPhase, VanillaInstallRequest, VanillaSources, VersionSource,
};
pub use rmcl_minecraft::{VersionManifest, VersionSummary};
pub use rmcl_net::ProgressStage as DownloadStage;
pub use task::{
    TaskEvent, TaskId, TaskKind, TaskManager, TaskPhase, TaskProgress, TaskSnapshot, TaskStatus,
};
pub use versions::{InstalledVersion, delete_installed_version, discover_installed_versions};
