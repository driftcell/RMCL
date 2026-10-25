use std::path::PathBuf;

use url::Url;

use crate::Sha1Hash;

/// A transport-independent request for one expected file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadRequest {
    pub sources: Vec<Url>,
    pub destination: PathBuf,
    pub sha1: Option<Sha1Hash>,
    pub size: Option<u64>,
}
