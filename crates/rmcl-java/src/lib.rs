//! Java runtime discovery, probing, and selection.

mod discovery;
mod error;
mod marker;
mod model;
mod mojang;
mod mojang_install;
mod probe;
mod runtime_path;
mod select;

pub use discovery::{JavaCandidate, JavaDetector, JavaDiscovery, JavaProbeFailure};
pub use error::JavaError;
pub use marker::{MARKER_SCHEMA_VERSION, MarkerFile, RuntimeMarker};
pub use model::{JavaRequirement, JavaRuntime, JavaRuntimeSource};
pub use mojang::{
    MOJANG_RUNTIME_INDEX_URL, MojangDownload, MojangFile, MojangFileDownloads, MojangFileType,
    MojangRuntimeEntry, MojangRuntimeIndex, MojangRuntimeManifest, MojangRuntimeVersion,
    SelectedRuntime, canonical_component, mojang_platform_key, runtime_components, select_runtime,
    select_runtime_entry,
};
pub use mojang_install::{MojangRuntimeInstaller, RuntimeInstallLock, UrlPolicy};
pub use probe::{ProbeConfig, parse_major_version, probe, probe_with_config};
pub use select::select_java;
