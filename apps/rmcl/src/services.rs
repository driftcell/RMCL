use std::{
    path::{Path, PathBuf},
    process::Command,
    rc::Rc,
};

use rmcl_app::{
    AppError, ContentInstaller, ForgeInstaller, InstanceCreator, JavaService, JavaServiceConfig,
    LoaderCatalog, MICROSOFT_KEYRING_SERVICE, MicrosoftAccountService, ModrinthClient,
    ServerIconCache, ServerStatusClient, SkinCache, TaskManager, VanillaInstaller, VersionCatalog,
};
use rmcl_auth::{MicrosoftAccount, OfflineAccount};
use rmcl_domain::{OperatingSystem, Platform};
use rmcl_net::DownloadConfig;
use rmcl_platform::KeyringSecretStore;
use rmcl_store::{
    CONFIG_SCHEMA_VERSION, ConfigStore, LauncherConfig, StoredAccountKind, StoredMicrosoftAccount,
    StoredOfflineAccount, default_config_path, default_java_store, default_minecraft_root,
};
use tokio::runtime::{Builder, Runtime};
use uuid::Uuid;

pub struct Services {
    pub runtime: Runtime,
    pub tasks: TaskManager,
    pub java: JavaService,
    pub platform: Platform,
    pub catalog: VersionCatalog,
    pub modrinth: ModrinthClient,
    pub loaders: LoaderCatalog,
    pub config_store: ConfigStore,
    pub microsoft_accounts: MicrosoftAccountService,
    pub skins: SkinCache,
    pub servers: ServerStatusClient,
    pub server_icons: ServerIconCache,
}

impl Services {
    pub fn new(config: &LauncherConfig, config_store: ConfigStore) -> Result<Rc<Self>, AppError> {
        let runtime = Builder::new_multi_thread()
            .enable_all()
            .thread_name("rmcl-core")
            .build()
            .map_err(|source| AppError::Io {
                operation: "create Tokio runtime",
                path: PathBuf::from("<runtime>"),
                source,
            })?;
        let installer = VanillaInstaller::official(DownloadConfig::default())?;
        let mut java_config = JavaServiceConfig::new(config.java_store.clone());
        java_config.auto_install = config.auto_install_java;
        let java = JavaService::new(java_config)?;
        let content = ContentInstaller::official(DownloadConfig::default())?;
        let loaders = LoaderCatalog::official()?;
        let forge = ForgeInstaller::new(DownloadConfig::default())?;
        let creator = InstanceCreator::new(installer.clone(), loaders.clone())
            .with_forge(forge, java.clone());
        let catalog = VersionCatalog::official(DownloadConfig::default())?;
        let modrinth = ModrinthClient::official()?;
        let tasks = TaskManager::with_full_and_modrinth(
            installer,
            java.clone(),
            content,
            creator,
            modrinth.clone(),
            256,
        );
        let secrets = std::sync::Arc::new(KeyringSecretStore::new(MICROSOFT_KEYRING_SERVICE)?);
        let microsoft_accounts = MicrosoftAccountService::new(secrets)?;
        let skin_cache_dir = config_store
            .path()
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("cache")
            .join("skins");
        let skins = SkinCache::new(skin_cache_dir);
        let server_icon_cache_dir = config_store
            .path()
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("cache")
            .join("servers");
        Ok(Rc::new(Self {
            runtime,
            tasks,
            java,
            platform: current_platform(),
            catalog,
            modrinth,
            loaders,
            config_store,
            microsoft_accounts,
            skins,
            servers: ServerStatusClient::default(),
            server_icons: ServerIconCache::new(server_icon_cache_dir),
        }))
    }
}

#[derive(Debug, Clone)]
pub struct Bootstrap {
    pub config: LauncherConfig,
    pub config_store: ConfigStore,
    pub config_warning: Option<String>,
    /// Whether startup normalization may be written back to the configuration.
    pub save_on_start: bool,
}

impl Bootstrap {
    pub fn from_args() -> Result<Option<Self>, String> {
        let mut arguments = std::env::args_os().skip(1);
        let mut config_path = default_config_path();
        let mut minecraft_root = None;
        let mut java_store = None;
        let mut version = None;
        let mut player = None;

        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("--help") | Some("-h") => return Ok(None),
                Some("--config") => {
                    config_path = PathBuf::from(next_value(&mut arguments, "--config")?);
                }
                Some("--minecraft-root") => {
                    minecraft_root = Some(PathBuf::from(next_value(
                        &mut arguments,
                        "--minecraft-root",
                    )?));
                }
                Some("--java-store") => {
                    java_store = Some(PathBuf::from(next_value(&mut arguments, "--java-store")?));
                }
                Some("--version") => {
                    version = Some(
                        next_value(&mut arguments, "--version")?
                            .into_string()
                            .map_err(|_| "--version must be valid UTF-8".to_owned())?,
                    );
                }
                Some("--player") => {
                    player = Some(
                        next_value(&mut arguments, "--player")?
                            .into_string()
                            .map_err(|_| "--player must be valid UTF-8".to_owned())?,
                    );
                }
                _ => {
                    return Err(format!(
                        "unsupported argument `{}`; use --help",
                        argument.to_string_lossy()
                    ));
                }
            }
        }

        let has_overrides = minecraft_root.is_some()
            || java_store.is_some()
            || version.is_some()
            || player.is_some();
        let config_existed = config_path.exists();
        let config_store = ConfigStore::new(config_path);
        let (mut config, config_warning) = match config_store.load() {
            Ok(config) => (config, None),
            Err(error) => (LauncherConfig::default(), Some(error.to_string())),
        };
        config.schema_version = CONFIG_SCHEMA_VERSION;

        if !config_existed && config.accounts.is_empty() && config.microsoft_accounts.is_empty() {
            select_offline_account(
                &mut config,
                OfflineAccount::new("Steve").map_err(|e| e.to_string())?,
            );
        }
        if let Some(root) = minecraft_root {
            config.minecraft_root = root;
        }
        if let Some(store) = java_store {
            config.java_store = store;
        }
        if let Some(version) = version {
            config.selected_version = Some(version);
        }
        if let Some(player) = player {
            select_offline_account(
                &mut config,
                OfflineAccount::new(player).map_err(|e| e.to_string())?,
            );
        }
        ensure_selected_account(&mut config);
        if config.minecraft_root.as_os_str().is_empty() {
            config.minecraft_root = default_minecraft_root();
        }
        if config.java_store.as_os_str().is_empty() {
            config.java_store = default_java_store();
        }

        Ok(Some(Self {
            config,
            config_store,
            config_warning: config_warning.clone(),
            save_on_start: should_save_on_start(
                config_existed,
                has_overrides,
                config_warning.as_deref(),
            ),
        }))
    }

    pub fn usage() -> &'static str {
        "RMCL\n\nOptions:\n  --config <PATH>          Launcher configuration (default: ~/.config/rmcl/config.toml)\n  --minecraft-root <PATH>  Override Minecraft repository for this run\n  --java-store <PATH>      Override managed Java store for this run\n  --version <ID>           Override the initially selected version\n  --player <NAME>          Add or select an offline player\n  -h, --help               Show this help"
    }
}

pub fn stored_account(account: &OfflineAccount) -> StoredOfflineAccount {
    StoredOfflineAccount {
        username: account.username().to_owned(),
        uuid: account.uuid().to_string(),
    }
}

pub fn load_account(stored: &StoredOfflineAccount) -> Option<OfflineAccount> {
    match Uuid::parse_str(&stored.uuid) {
        Ok(uuid) => OfflineAccount::with_uuid(stored.username.clone(), uuid).ok(),
        // A damaged UUID must not make an otherwise usable offline profile disappear.
        Err(_) => OfflineAccount::new(stored.username.clone()).ok(),
    }
}

pub fn stored_microsoft_account(account: &MicrosoftAccount) -> StoredMicrosoftAccount {
    StoredMicrosoftAccount {
        username: account.username().to_owned(),
        uuid: account.uuid().to_string(),
        skin_url: account.skin_url().map(str::to_owned),
    }
}

pub fn load_microsoft_account(stored: &StoredMicrosoftAccount) -> Option<MicrosoftAccount> {
    Uuid::parse_str(&stored.uuid).ok().and_then(|uuid| {
        MicrosoftAccount::new(stored.username.clone(), uuid)
            .ok()
            .map(|account| account.with_skin_url(stored.skin_url.clone()))
    })
}

fn select_offline_account(config: &mut LauncherConfig, account: OfflineAccount) {
    config
        .accounts
        .retain(|stored| stored.username != account.username());
    config.accounts.push(stored_account(&account));
    config.selected_account = Some(account.username().to_owned());
    config.selected_account_kind = StoredAccountKind::Offline;
}

fn ensure_selected_account(config: &mut LauncherConfig) {
    let exists = match config.selected_account_kind {
        StoredAccountKind::Offline => config.selected_account.as_ref().is_some_and(|selected| {
            config
                .accounts
                .iter()
                .any(|account| account.username == *selected)
        }),
        StoredAccountKind::Microsoft => config.selected_account.as_ref().is_some_and(|selected| {
            config
                .microsoft_accounts
                .iter()
                .any(|account| account.username == *selected)
        }),
    };
    if exists {
        return;
    }
    if let Some(account) = config.accounts.first() {
        config.selected_account = Some(account.username.clone());
        config.selected_account_kind = StoredAccountKind::Offline;
    } else if let Some(account) = config.microsoft_accounts.first() {
        config.selected_account = Some(account.username.clone());
        config.selected_account_kind = StoredAccountKind::Microsoft;
    } else {
        config.selected_account = None;
        config.selected_account_kind = StoredAccountKind::Offline;
    }
}

fn should_save_on_start(config_existed: bool, has_overrides: bool, warning: Option<&str>) -> bool {
    warning.is_none() && (!config_existed || !has_overrides)
}

fn next_value(
    arguments: &mut impl Iterator<Item = std::ffi::OsString>,
    option: &str,
) -> Result<std::ffi::OsString, String> {
    arguments
        .next()
        .ok_or_else(|| format!("{option} requires a value"))
}

/// Opens a local file or directory with the platform's default handler.
pub fn open_path(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(windows)]
    let program = "explorer";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";

    Command::new(program).arg(path).spawn().map(|_| ())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_documents_config_override() {
        assert!(Bootstrap::usage().contains("--config"));
        assert!(Bootstrap::usage().contains("--minecraft-root"));
    }

    #[test]
    fn startup_writes_do_not_persist_command_line_overrides() {
        assert!(should_save_on_start(false, true, None));
        assert!(!should_save_on_start(true, true, None));
        assert!(should_save_on_start(true, false, None));
        assert!(!should_save_on_start(false, false, Some("broken config")));
    }

    #[test]
    fn stored_accounts_round_trip_the_derived_uuid() {
        let account = OfflineAccount::new("Steve").unwrap();
        let stored = stored_account(&account);
        let loaded = load_account(&stored).unwrap();

        assert_eq!(loaded.username(), account.username());
        assert_eq!(loaded.uuid(), account.uuid());
    }

    #[test]
    fn damaged_uuid_falls_back_to_the_offline_identity() {
        let stored = StoredOfflineAccount {
            username: "Alex".to_owned(),
            uuid: "not-a-uuid".to_owned(),
        };

        assert_eq!(
            load_account(&stored).unwrap().uuid(),
            OfflineAccount::new("Alex").unwrap().uuid()
        );
    }

    #[test]
    fn microsoft_accounts_round_trip_their_profile_uuid() {
        let uuid = Uuid::parse_str("986dec87-b7ec-47ff-89ff-033fdb95c4b5").unwrap();
        let account = MicrosoftAccount::new("HowDoesAuthWork", uuid).unwrap();
        let stored = stored_microsoft_account(&account);
        let loaded = load_microsoft_account(&stored).unwrap();

        assert_eq!(loaded.username(), account.username());
        assert_eq!(loaded.uuid(), account.uuid());
    }

    #[test]
    fn damaged_microsoft_uuid_disables_the_account() {
        let stored = StoredMicrosoftAccount {
            username: "HowDoesAuthWork".to_owned(),
            uuid: "not-a-uuid".to_owned(),
            skin_url: None,
        };

        assert!(load_microsoft_account(&stored).is_none());
    }

    #[test]
    fn missing_selection_falls_back_to_an_available_account() {
        let mut config = LauncherConfig::default();
        config.microsoft_accounts.push(StoredMicrosoftAccount {
            username: "HowDoesAuthWork".to_owned(),
            uuid: "986dec87-b7ec-47ff-89ff-033fdb95c4b5".to_owned(),
            skin_url: None,
        });

        ensure_selected_account(&mut config);

        assert_eq!(config.selected_account.as_deref(), Some("HowDoesAuthWork"));
        assert_eq!(config.selected_account_kind, StoredAccountKind::Microsoft);
    }
}
