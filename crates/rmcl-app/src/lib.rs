//! Headless application use cases and task orchestration.

mod accounts;
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
pub use catalog::VersionCatalog;
pub use content::{
    ContentBatchOutcome, ContentDownloadProgress, ContentGraphInstallRequest, ContentInstallEvent,
    ContentInstallEventSender, ContentInstallOutcome, ContentInstallPhase, ContentInstallPreflight,
    ContentInstallRequest, ContentInstaller, ContentPreflightItem, ContentPreflightStatus,
    ModMetadataCache, ModMetadataFetch, delete_instance_content,
    delete_instance_content_with_guard, import_instance_content,
    import_instance_content_with_guard, preflight_content_install, resolve_content_install_batch,
    resolve_content_install_order, scan_instance_content, set_instance_content_enabled,
    set_instance_content_enabled_with_guard,
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
pub use rmcl_content::{
    ConflictPolicy, ContentKind, ContentLibrary, ContentSource, ContentWarning, InstalledMod,
    InstalledModIndex, LocalContent, ModDependency, ModMetadata, ModMetadataError,
    ModMetadataLoader, ModrinthClient, ModrinthConfig, RemoteContentError, RemoteDependency,
    RemoteDependencyKind, RemoteFile, RemoteProject, RemoteSearchPage, RemoteSearchQuery,
    RemoteSort, RemoteVersion, read_mod_metadata, recommended_version, scan_installed_mods,
};
pub use rmcl_instance::{
    InstanceLayout, InstanceLoader, InstanceManifest, IsolationMode, LoaderKind, RepositoryPath,
};
pub use rmcl_java::{JavaDiscovery, JavaRuntime, JavaRuntimeSource};
pub use rmcl_minecraft::{VersionManifest, VersionSummary};
pub use rmcl_net::ProgressStage as DownloadStage;
pub use servers::{ServerIconCache, ServerPingError, ServerStatus, ServerStatusClient};
pub use services::AppContext;
pub use skin_library::{SkinLibrary, SkinLibraryEntry, SkinLibraryError};
pub use skin_preview::SkinPreviewRenderer;
pub use skin_source::{
    MAX_SKIN_SOURCE_BYTES, MAX_SKIN_UPLOAD_BYTES, PreparedSkin, SkinSourceError, SkinUrlClient,
    load_skin_file, prepare_skin,
};
pub use skins::{SkinCache, SkinError, SkinTexture};
pub use task::{
    TaskEvent, TaskId, TaskKind, TaskManager, TaskPhase, TaskProgress, TaskSnapshot, TaskStatus,
    TaskSubject,
};
pub use update::{
    UpdateAsset, UpdateClient, UpdateError, UpdatePackage, UpdatePlan, UpdateProgress,
    UpdateRelease, apply_update, spawn_apply_helper,
};
pub use versions::{InstalledVersion, delete_installed_version, discover_installed_versions};
