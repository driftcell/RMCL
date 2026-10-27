//! Shared primitives for RMCL domain services.

mod checksum;
mod download;
mod extraction;
mod mirror;
mod platform;
mod server;

pub use checksum::{ParseSha1Error, Sha1Hash};
pub use download::DownloadRequest;
pub use extraction::ExtractionPlan;
pub use mirror::MirrorSource;
pub use platform::{Architecture, OperatingSystem, Platform};
pub use server::{DEFAULT_MINECRAFT_PORT, ServerAddress, ServerAddressError};
