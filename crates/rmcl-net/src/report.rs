use rmcl_domain::DownloadRequest;

use crate::DownloadError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStatus {
    Downloaded { bytes: u64 },
    Skipped { bytes: u64 },
}

impl DownloadStatus {
    pub fn bytes(self) -> u64 {
        match self {
            Self::Downloaded { bytes } | Self::Skipped { bytes } => bytes,
        }
    }
}

#[derive(Debug)]
pub struct DownloadOutcome {
    pub request: DownloadRequest,
    pub status: DownloadStatus,
}

#[derive(Debug)]
pub struct DownloadFailure {
    pub request: DownloadRequest,
    pub error: DownloadError,
}

#[derive(Debug, Default)]
pub struct DownloadReport {
    pub outcomes: Vec<DownloadOutcome>,
    pub failures: Vec<DownloadFailure>,
    pub cancelled: bool,
}

impl DownloadReport {
    pub fn is_success(&self) -> bool {
        !self.cancelled && self.failures.is_empty()
    }
}
