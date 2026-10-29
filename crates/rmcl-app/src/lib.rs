//! Headless application use cases and task orchestration.

mod accounts;
mod authlib_accounts;
mod authlib_injector;
mod catalog;
mod content;
mod error;
mod featured;
mod forge;
mod game;
mod install;
mod instance_create;
mod instances;
mod java;
mod loader;
mod locks;
mod model;
mod remote_icons;
mod servers;
mod services;
mod skin_library;
mod skin_preview;
mod skin_source;
mod skins;
mod task;
mod update;
mod versions;

pub use accounts::{
    MICROSOFT_KEYRING_SERVICE, MicrosoftAccountService, MicrosoftAccountServiceError,
    ResolvedMicrosoftAccount,
};
pub use authlib_accounts::{
    AUTHLIB_KEYRING_SERVICE, AuthlibAccountService, AuthlibAccountServiceError,
    ResolvedAuthlibAccount,
};
pub use authlib_injector::{AuthlibInjectorArtifact, AuthlibInjectorError, AuthlibInjectorService};
pub use catalog::VersionCatalog;
pub use content::{
    ContentBatchOutcome, ContentDownloadProgress, ContentGraphInstallRequest, ContentInspection,
    ContentInstallEvent, ContentInstallEventSender, ContentInstallOutcome, ContentInstallPhase,
    ContentInstallPreflight, ContentInstallRequest, ContentInstaller, ContentPreflightItem,
    ContentPreflightStatus, ModMetadataCache, ModMetadataFetch, ResourcePackInspection,
    delete_instance_content, delete_instance_content_with_guard, import_instance_content,
    import_instance_content_with_guard, inspect_content, move_instance_resource_pack,
    preflight_content_install, read_instance_shader_pack, resolve_content_install_batch,
    resolve_content_install_order, resource_pack_format, scan_and_inspect_instance_content,
    scan_instance_content, set_instance_content_enabled, set_instance_content_enabled_with_guard,
    set_instance_shader_pack,
};
pub use error::{AppError, InstallDownloadFailure};
pub use featured::{FeaturedServer, FeaturedServerError, featured_servers};
pub use forge::{ForgeInstallOutcome, ForgeInstallRequest, ForgeInstaller};
pub use game::{
    GameLaunchEvent, GameLaunchEventSender, GameLaunchOutcome, GameLaunchRequest, GameLauncher,
};
pub use install::VanillaInstaller;
pub use instance_create::{
    CreateInstanceRequest, InstanceCreateEvent, InstanceCreateEventSender, InstanceCreateOutcome,
    InstanceCreatePhase, InstanceCreator, LoaderSelection,
};
pub use instances::{
    migrate_instance_to_isolated, migrate_instance_to_isolated_with_guard, rename_instance,
    rename_instance_with_guard, set_instance_java, set_instance_java_with_guard,
    set_instance_launch_settings, set_instance_launch_settings_with_guard,
};
pub use java::{
    JavaEvent, JavaEventSender, JavaSelection, JavaSelectionOrigin, JavaService, JavaServiceConfig,
};
pub use loader::{LoaderCatalog, LoaderCatalogConfig, LoaderVersion};
pub use locks::{RepositoryWriteGuard, try_lock_repository};
pub use model::{
    DownloadSummary, InstallDownloadProgress, InstallEvent, InstallEventSender, InstallOutcome,
    InstallPhase, VanillaInstallRequest, VanillaSources, VersionSource,
};
pub use remote_icons::{MAX_REMOTE_ICON_BYTES, RemoteIconCache, RemoteIconError};
pub use rmcl_content::{
    ConflictPolicy, ContentKind, ContentLibrary, ContentSource, ContentWarning, InstalledMod,
    InstalledModIndex, LocalContent, ModDependency, ModMetadata, ModMetadataError,
    ModMetadataLoader, ModrinthClient, ModrinthConfig, PackFormat, PackFormatRange,
    RemoteContentError, RemoteDependency, RemoteDependencyKind, RemoteFile, RemoteProject,
    RemoteSearchPage, RemoteSearchQuery, RemoteSort, RemoteVersion, ResourcePackCompatibility,
    ResourcePackMetadata, ResourcePackOrder, ShaderLoader, ShaderPackMetadata, ShaderPackSelection,
    read_client_resource_pack_format, read_mod_metadata, read_resource_pack_metadata,
    read_shader_pack_metadata, read_shader_pack_selection, recommended_version,
    replace_content_file, scan_installed_mods, set_shader_pack_selection,
};
pub use rmcl_instance::{
    InstanceLayout, InstanceLoader, InstanceManifest, IsolationMode, LoaderKind, RepositoryPath,
};
pub use rmcl_java::{JavaDiscovery, JavaRuntime, JavaRuntimeSource};
pub use rmcl_minecraft::{VersionManifest, VersionSummary};
pub use rmcl_net::ProgressStage as DownloadStage;
pub use servers::{
    ServerBookmarkError, ServerIconCache, ServerPingError, ServerStatus, ServerStatusClient,
    remove_server_bookmark, upsert_server_bookmark,
};
pub use services::AppContext;
pub use skin_library::{SkinLibrary, SkinLibraryEntry, SkinLibraryError};
pub use skin_preview::SkinPreviewRenderer;
pub use skin_source::{
    MAX_SKIN_SOURCE_BYTES, MAX_SKIN_UPLOAD_BYTES, PreparedSkin, SkinSourceError, SkinUrlClient,
    load_skin_file, prepare_skin,
};
pub use skins::{SkinCache, SkinError, SkinTexture};
pub use task::{
    JavaEnsureRequest, TaskEvent, TaskId, TaskKind, TaskManager, TaskPhase, TaskProgress,
    TaskSnapshot, TaskStatus, TaskSubject,
};
pub use update::{
    UpdateAsset, UpdateClient, UpdateError, UpdatePackage, UpdatePlan, UpdateProgress,
    UpdateRelease, apply_update, spawn_apply_helper,
};
pub use versions::{
    InstalledVersion, delete_installed_version, delete_installed_version_with_guard,
    discover_installed_versions,
};
