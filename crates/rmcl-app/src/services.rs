use std::{path::PathBuf, sync::Arc};

use rmcl_domain::{OperatingSystem, Platform};
use rmcl_net::{DownloadConfig, MirrorHealth, MirrorPreference};
use rmcl_platform::KeyringSecretStore;
use rmcl_store::{
    ConfigStore, DownloadSourcePreference, LauncherConfig, default_config_path, default_java_store,
    default_minecraft_root,
};
use tokio::runtime::{Builder, Runtime};

use crate::{
    AppError, ContentInstaller, ForgeInstaller, InstanceCreator, JavaService, JavaServiceConfig,
    LoaderCatalog, MICROSOFT_KEYRING_SERVICE, MicrosoftAccountService, ModMetadataCache,
    ModrinthClient, RemoteIconCache, ServerIconCache, ServerStatusClient, SkinCache, SkinLibrary,
    SkinUrlClient, TaskManager, UpdateClient, VanillaInstaller, VersionCatalog,
};

/// Shared headless services used by the desktop UI and external protocols.
pub struct AppContext {
    pub runtime: Runtime,
    pub tasks: TaskManager,
    pub java: JavaService,
    pub platform: Platform,
    pub catalog: VersionCatalog,
    pub modrinth: ModrinthClient,
    pub loaders: LoaderCatalog,
    pub config_store: ConfigStore,
    pub minecraft_root: PathBuf,
    pub java_store: PathBuf,
    pub microsoft_accounts: MicrosoftAccountService,
    pub skins: SkinCache,
    pub skin_library: SkinLibrary,
    pub skin_urls: SkinUrlClient,
    pub servers: ServerStatusClient,
    pub server_icons: ServerIconCache,
    pub remote_icons: RemoteIconCache,
    pub mod_metadata: ModMetadataCache,
    pub updates: UpdateClient,
    pub mirror: Arc<MirrorHealth>,
}

impl AppContext {
    pub fn from_default_config() -> Result<Arc<Self>, AppError> {
        Self::from_config_path(default_config_path()?)
    }

    pub fn from_config_path(path: PathBuf) -> Result<Arc<Self>, AppError> {
        let config_store = ConfigStore::new(path);
        let mut config = config_store.load()?;
        if config.minecraft_root.as_os_str().is_empty() {
            config.minecraft_root = default_minecraft_root()?;
        }
        if config.java_store.as_os_str().is_empty() {
            config.java_store = default_java_store()?;
        }
        Self::new(&config, config_store)
    }

    pub fn new(config: &LauncherConfig, config_store: ConfigStore) -> Result<Arc<Self>, AppError> {
        let runtime = Builder::new_multi_thread()
            .enable_all()
            .thread_name("rmcl-core")
            .build()
            .map_err(|source| AppError::Io {
                operation: "create Tokio runtime",
                path: PathBuf::from("<runtime>"),
                source,
            })?;
        let mirror = Arc::new(MirrorHealth::new(mirror_preference(config.download_source)));
        let installer =
            VanillaInstaller::official(DownloadConfig::default())?.with_mirror(mirror.clone());
        let mut java_config = JavaServiceConfig::new(config.java_store.clone());
        java_config.auto_install = config.auto_install_java;
        java_config.mirror = Some(mirror.clone());
        let java = JavaService::new(java_config)?;
        let content = ContentInstaller::official(DownloadConfig::default())?;
        let loaders = LoaderCatalog::official()?.with_mirror(mirror.clone());
        let forge = ForgeInstaller::new(DownloadConfig::default())?.with_mirror(mirror.clone());
        let creator = InstanceCreator::new(installer.clone(), loaders.clone())
            .with_forge(forge, java.clone());
        let catalog =
            VersionCatalog::official(DownloadConfig::default())?.with_mirror(mirror.clone());
        let modrinth = ModrinthClient::official()?;
        let tasks = TaskManager::with_full_and_modrinth(
            installer,
            java.clone(),
            content,
            creator,
            modrinth.clone(),
            256,
        );
        let secrets = Arc::new(KeyringSecretStore::new(MICROSOFT_KEYRING_SERVICE)?);
        let microsoft_accounts = MicrosoftAccountService::new(secrets)?;
        let config_dir = config_store
            .path()
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let skins = SkinCache::new(config_dir.join("cache").join("skins"));
        let skin_library = SkinLibrary::new(config_dir.join("skins").join("library"));
        let skin_urls = SkinUrlClient::new().map_err(AppError::SkinSource)?;
        let server_icon_cache_dir = config_dir.join("cache").join("servers");
        let remote_icon_cache_dir = config_dir.join("cache").join("remote-icons");
        let mod_metadata_cache_dir = config_dir.join("cache").join("mod-metadata");
        let mod_metadata =
            ModMetadataCache::new(DownloadConfig::default(), mod_metadata_cache_dir)?;
        let updates =
            UpdateClient::github().map_err(|error| AppError::Update(error.to_string()))?;
        Ok(Arc::new(Self {
            runtime,
            tasks,
            java,
            platform: current_platform(),
            catalog,
            modrinth,
            loaders,
            config_store,
            minecraft_root: config.minecraft_root.clone(),
            java_store: config.java_store.clone(),
            microsoft_accounts,
            skins,
            skin_library,
            skin_urls,
            servers: ServerStatusClient::default(),
            server_icons: ServerIconCache::new(server_icon_cache_dir),
            remote_icons: RemoteIconCache::new(remote_icon_cache_dir)?,
            mod_metadata,
            updates,
            mirror,
        }))
    }
}

fn mirror_preference(value: DownloadSourcePreference) -> MirrorPreference {
    match value {
        DownloadSourcePreference::Auto => MirrorPreference::Auto,
        DownloadSourcePreference::Official => MirrorPreference::Official,
        DownloadSourcePreference::Mirror => MirrorPreference::Mirror,
    }
}

fn current_platform() -> Platform {
    let os = if cfg!(windows) {
        OperatingSystem::Windows
    } else if cfg!(target_os = "macos") {
        OperatingSystem::MacOs
    } else {
        OperatingSystem::Linux
    };
    Platform::new(
        os,
        sysinfo::System::os_version().unwrap_or_default(),
        std::env::consts::ARCH,
    )
}
