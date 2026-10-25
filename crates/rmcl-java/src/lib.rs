//! Java runtime discovery, probing, and selection.

mod discovery;
mod error;
mod model;
mod probe;
mod select;

pub use discovery::{JavaCandidate, JavaDetector, JavaDiscovery, JavaProbeFailure};
pub use error::JavaError;
pub use model::{JavaRequirement, JavaRuntime, JavaRuntimeSource};
pub use probe::{ProbeConfig, parse_major_version, probe, probe_with_config};
pub use select::select_java;
