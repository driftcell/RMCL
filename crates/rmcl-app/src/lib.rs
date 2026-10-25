//! Headless application use cases and task orchestration.

mod error;
mod install;
mod java;
mod model;
mod task;

pub use error::{AppError, InstallDownloadFailure};
pub use install::VanillaInstaller;
pub use java::{
    JavaEvent, JavaEventSender, JavaSelection, JavaSelectionOrigin, JavaService, JavaServiceConfig,
};
pub use model::{
    DownloadSummary, InstallDownloadProgress, InstallEvent, InstallEventSender, InstallOutcome,
    InstallPhase, VanillaInstallRequest, VanillaSources, VersionSource,
};
pub use rmcl_net::ProgressStage as DownloadStage;
pub use task::{TaskEvent, TaskId, TaskKind, TaskManager, TaskProgress, TaskSnapshot, TaskStatus};
