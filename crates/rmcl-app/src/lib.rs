//! Headless application use cases and task orchestration.

mod error;
mod install;
mod model;

pub use error::{AppError, InstallDownloadFailure};
pub use install::VanillaInstaller;
pub use model::{
    DownloadSummary, InstallDownloadProgress, InstallEvent, InstallEventSender, InstallOutcome,
    InstallPhase, VanillaInstallRequest, VanillaSources, VersionSource,
};
pub use rmcl_net::ProgressStage as DownloadStage;
