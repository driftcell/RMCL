//! Local Minecraft content discovery and instance-scoped management.

mod error;
mod kind;
mod local;
mod metadata;
mod model;
mod options;
mod remote;
mod version_req;

pub use error::ContentError;
pub use kind::ContentKind;
pub use local::{
    ConflictPolicy, delete_content, import_file, scan_kind, scan_library, set_content_enabled,
};
pub use metadata::{
    InstalledMod, InstalledModIndex, ModDependency, ModMetadata, ModMetadataError,
    ModMetadataLoader, read_mod_metadata, scan_installed_mods,
};
pub use model::{ContentLibrary, ContentScan, ContentWarning, LocalContent};
pub use remote::{
    ContentSource, ModrinthClient, ModrinthConfig, RemoteContentError, RemoteDependency,
    RemoteDependencyKind, RemoteFile, RemoteProject, RemoteSearchPage, RemoteSearchQuery,
    RemoteSort, RemoteVersion, recommended_version,
};
pub use version_req::{
    VersionRequirement, VersionRequirementError, VersionStyle, version_satisfies,
};
