//! Local Minecraft content discovery and instance-scoped management.

mod error;
mod kind;
mod local;
mod metadata;
mod model;
mod options;
mod pack;
mod properties;
mod remote;
mod shader;
mod version_req;

pub use error::ContentError;
pub use kind::ContentKind;
pub use local::{
    ConflictPolicy, delete_content, import_file, move_resource_pack, read_resource_pack_order,
    replace_content_file, scan_kind, scan_library, set_content_enabled,
};
pub use metadata::{
    InstalledMod, InstalledModIndex, ModDependency, ModMetadata, ModMetadataError,
    ModMetadataLoader, ShaderLoader, read_mod_metadata, scan_installed_mods,
};
pub use model::{ContentLibrary, ContentScan, ContentWarning, LocalContent};
pub use options::ResourcePackOrder;
pub use pack::{
    PackFormat, PackFormatRange, ResourcePackCompatibility, ResourcePackMetadata,
    ShaderPackMetadata, read_client_resource_pack_format, read_resource_pack_metadata,
    read_shader_pack_metadata,
};
pub use remote::{
    ContentSource, ModrinthClient, ModrinthConfig, RemoteContentError, RemoteDependency,
    RemoteDependencyKind, RemoteFile, RemoteProject, RemoteSearchPage, RemoteSearchQuery,
    RemoteSort, RemoteVersion, recommended_version,
};
pub use shader::{ShaderPackSelection, read_shader_pack_selection, set_shader_pack_selection};
pub use version_req::{
    VersionRequirement, VersionRequirementError, VersionStyle, version_satisfies,
};
