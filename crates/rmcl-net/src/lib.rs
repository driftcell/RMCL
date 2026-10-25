//! Async HTTP download execution with integrity checks and bounded concurrency.

mod config;
mod error;
mod executor;
mod integrity;
mod progress;
mod report;

pub use config::DownloadConfig;
pub use error::{DownloadError, SourceFailure};
pub use executor::DownloadExecutor;
pub use progress::{ProgressEvent, ProgressSender, ProgressStage, progress_channel};
pub use report::{DownloadFailure, DownloadOutcome, DownloadReport, DownloadStatus};
