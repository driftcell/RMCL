use std::path::PathBuf;

use crate::{InstallPhase, task::TaskId};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallDownloadFailure {
    pub destination: PathBuf,
    pub message: String,
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error("operation was cancelled")]
    Cancelled,
    #[error("task {existing} is already installing this repository version")]
    TaskAlreadyRunning { existing: TaskId },
    #[error("game launch requires a configured Java service")]
    JavaNotConfigured,
    #[error("no compatible Java runtime for {requirement:?}; probe failures: {failures:?}")]
    JavaNotFound {
        requirement: rmcl_java::JavaRequirement,
        failures: Vec<String>,
    },
    #[error("invalid application request: {0}")]
    InvalidRequest(String),
    #[error("version `{0}` was not found in the version manifest")]
    VersionNotFound(String),
    #[error("version JSON ID `{actual}` does not match requested ID `{expected}`")]
    VersionIdMismatch { expected: String, actual: String },
    #[error(
        "version `{version}` is incomplete and needs repair ({missing_count} required files missing; first: {first_missing})"
    )]
    InstallationIncomplete {
        version: String,
        missing_count: usize,
        first_missing: String,
    },
    #[error("{phase:?} download failed: {source}")]
    Download {
        phase: InstallPhase,
        #[source]
        source: rmcl_net::DownloadError,
    },
    #[error("{phase:?} downloads failed")]
    DownloadBatch {
        phase: InstallPhase,
        failures: Vec<InstallDownloadFailure>,
    },
    #[error("failed to {operation} `{path}`: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("JSON file `{path}` is not UTF-8: {source}")]
    JsonUtf8 {
        path: PathBuf,
        #[source]
        source: std::string::FromUtf8Error,
    },
    #[error("native extraction failed for `{path}`: {source}")]
    Extraction {
        path: PathBuf,
        #[source]
        source: rmcl_archive::ExtractionError,
    },
    #[error("blocking application task failed: {0}")]
    BlockingTask(String),
    #[error(transparent)]
    Metadata(#[from] rmcl_minecraft::MetadataError),
    #[error(transparent)]
    InstallPlan(#[from] rmcl_install::InstallPlanError),
    #[error(transparent)]
    Layout(#[from] rmcl_instance::LayoutError),
    #[error(transparent)]
    Instance(#[from] rmcl_instance::InstanceError),
    #[error(transparent)]
    Checksum(#[from] rmcl_domain::ParseSha1Error),
    #[error(transparent)]
    DownloadConfiguration(#[from] rmcl_net::DownloadError),
    #[error(transparent)]
    SourceUrl(#[from] url::ParseError),
    #[error(transparent)]
    Java(#[from] rmcl_java::JavaError),
    #[error(transparent)]
    Launch(#[from] rmcl_launch::LaunchError),
    #[error("Microsoft authentication is unavailable: {0}")]
    MicrosoftAuth(#[from] rmcl_auth::MicrosoftAuthError),
    #[error("system credential storage is unavailable: {0}")]
    SecretStore(#[from] rmcl_platform::SecretStoreError),
}
