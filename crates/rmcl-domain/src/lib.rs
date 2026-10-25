//! Shared primitives for RMCL domain services.

mod checksum;
mod download;
mod extraction;
mod platform;

pub use checksum::{ParseSha1Error, Sha1Hash};
pub use download::DownloadRequest;
pub use extraction::ExtractionPlan;
pub use platform::{Architecture, OperatingSystem, Platform};
