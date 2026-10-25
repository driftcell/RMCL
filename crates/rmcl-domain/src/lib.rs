//! Shared primitives for RMCL domain services.

mod checksum;
mod download;

pub use checksum::{ParseSha1Error, Sha1Hash};
pub use download::DownloadRequest;
