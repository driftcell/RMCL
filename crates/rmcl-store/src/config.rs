use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::StoreError;

pub const CONFIG_SCHEMA_VERSION: u32 = 8;

/// Launcher-owned settings. This file never replaces or rewrites HMCL configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LauncherConfig {
    pub schema_version: u32,
    pub minecraft_root: PathBuf,
    pub java_store: PathBuf,
    pub selected_version: Option<String>,
    pub selected_account: Option<String>,
    pub selected_account_kind: StoredAccountKind,
    pub accounts: Vec<StoredOfflineAccount>,
    pub microsoft_accounts: Vec<StoredMicrosoftAccount>,
    pub servers: Vec<StoredServer>,
    pub auto_install_java: bool,
    pub auto_check_updates: bool,
    pub download_source: DownloadSourcePreference,
    pub launcher_visibility: LauncherVisibility,
    pub agent_provider: String,
    pub agent_model: String,
    pub agent_base_url: Option<String>,
    pub agent_proxy: Option<String>,
    pub appearance: AppearanceConfig,
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            minecraft_root: PathBuf::new(),
            java_store: PathBuf::new(),
            selected_version: None,
            selected_account: None,
            selected_account_kind: StoredAccountKind::Offline,
            accounts: Vec::new(),
            microsoft_accounts: Vec::new(),
            servers: Vec::new(),
            auto_install_java: true,
            auto_check_updates: true,
            download_source: DownloadSourcePreference::default(),
            launcher_visibility: LauncherVisibility::default(),
            agent_provider: "kilo".to_owned(),
            agent_model: String::new(),
            agent_base_url: None,
            agent_proxy: None,
            appearance: AppearanceConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredOfflineAccount {
    pub username: String,
    pub uuid: String,
}

/// Account metadata that never contains tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredMicrosoftAccount {
    pub username: String,
    pub uuid: String,
    #[serde(default)]
    pub skin_url: Option<String>,
}

/// A launcher-owned server bookmark. The address remains a string so one bad
/// entry cannot make the whole configuration unreadable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredServer {
    pub id: String,
    pub name: String,
    pub address: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StoredAccountKind {
    #[default]
    Offline,
    Microsoft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AccentColor {
    #[default]
    Emerald,
    Ocean,
    Violet,
    Amber,
    Rose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DownloadSourcePreference {
    /// Probe the domestic mirror and use it when it is healthy.
    #[default]
    Auto,
    /// Use Mojang's official endpoints only.
    Official,
    /// Prefer the domestic mirror, falling back to official per request.
    Mirror,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LauncherVisibility {
    #[default]
    KeepVisible,
    HideWhileRunning,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceConfig {
    pub theme: ThemePreference,
    pub accent: AccentColor,
}

/// Reads and atomically writes one launcher configuration file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn discover() -> Result<Self, StoreError> {
        Ok(Self::new(default_config_path()?))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("logs")
    }

    /// Loads the configuration, returning defaults when the file does not exist.
    ///
    /// A malformed or newer file is reported as an error and left untouched so a
    /// transient read failure cannot silently discard user settings.
    pub fn load(&self) -> Result<LauncherConfig, StoreError> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(LauncherConfig::default());
            }
            Err(source) => {
                return Err(StoreError::Io {
                    operation: "read launcher configuration",
                    path: self.path.clone(),
                    source,
                });
            }
        };
        let config: LauncherConfig = toml::from_str(&text).map_err(|source| StoreError::Toml {
            path: self.path.clone(),
            source,
        })?;
        if config.schema_version == 0 || config.schema_version > CONFIG_SCHEMA_VERSION {
            return Err(StoreError::UnsupportedSchema {
                found: config.schema_version,
                supported: CONFIG_SCHEMA_VERSION,
            });
        }
        let mut config = config;
        // Schema 1 predates Microsoft account metadata, schema 3 adds server
        // bookmarks, schema 4 adds update checks, schema 5 adds the download
        // source, and schema 6 adds disabled-by-default loopback MCP settings.
        // Every new field has a safe default.
        if config.schema_version < CONFIG_SCHEMA_VERSION {
            config.schema_version = CONFIG_SCHEMA_VERSION;
        }
        Ok(config)
    }

    /// Writes the whole configuration through a synced temporary file.
    pub fn save(&self, config: &LauncherConfig) -> Result<(), StoreError> {
        let parent = self.path.parent().ok_or_else(|| StoreError::Io {
            operation: "find configuration directory",
            path: self.path.clone(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "configuration path has no parent",
            ),
        })?;
        fs::create_dir_all(parent).map_err(|source| StoreError::Io {
            operation: "create configuration directory",
            path: parent.to_path_buf(),
            source,
        })?;
        let text = toml::to_string_pretty(config).map_err(|source| StoreError::TomlSerialize {
            path: self.path.clone(),
            source,
        })?;
        let mut temporary = NamedTempFile::new_in(parent).map_err(|source| StoreError::Io {
            operation: "create configuration temporary file",
            path: parent.to_path_buf(),
            source,
        })?;
        temporary
            .write_all(text.as_bytes())
            .map_err(|source| StoreError::Io {
                operation: "write configuration temporary file",
                path: temporary.path().to_path_buf(),
                source,
            })?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|source| StoreError::Io {
                operation: "sync configuration temporary file",
                path: temporary.path().to_path_buf(),
                source,
            })?;
        temporary
            .persist(&self.path)
            .map_err(|error| StoreError::Io {
                operation: "publish launcher configuration",
                path: self.path.clone(),
                source: error.error,
            })?;
        Ok(())
    }
}

pub fn home_directory() -> Option<PathBuf> {
    dirs::home_dir()
}

pub fn default_config_dir() -> Result<PathBuf, StoreError> {
    dirs::config_dir()
        .map(|path| path.join("rmcl"))
        .ok_or(StoreError::DirectoryUnavailable {
            directory: "configuration",
        })
}

pub fn default_minecraft_root() -> Result<PathBuf, StoreError> {
    if cfg!(windows) {
        dirs::data_dir().map(|path| path.join(".minecraft")).ok_or(
            StoreError::DirectoryUnavailable {
                directory: "application data",
            },
        )
    } else if cfg!(target_os = "macos") {
        dirs::data_dir().map(|path| path.join("minecraft")).ok_or(
            StoreError::DirectoryUnavailable {
                directory: "application data",
            },
        )
    } else {
        home_directory()
            .map(|path| path.join(".minecraft"))
            .ok_or(StoreError::DirectoryUnavailable { directory: "home" })
    }
}

pub fn default_java_store() -> Result<PathBuf, StoreError> {
    home_directory()
        .map(|path| path.join(".rmcl").join("java"))
        .ok_or(StoreError::DirectoryUnavailable { directory: "home" })
}

pub fn default_config_path() -> Result<PathBuf, StoreError> {
    Ok(default_config_dir()?.join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn account(username: &str, uuid: &str) -> StoredOfflineAccount {
        StoredOfflineAccount {
            username: username.to_owned(),
            uuid: uuid.to_owned(),
        }
    }

    fn microsoft_account(username: &str, uuid: &str) -> StoredMicrosoftAccount {
        StoredMicrosoftAccount {
            username: username.to_owned(),
            uuid: uuid.to_owned(),
            skin_url: Some("http://textures.minecraft.net/texture/abc".to_owned()),
        }
    }

    fn server(id: &str, name: &str, address: &str) -> StoredServer {
        StoredServer {
            id: id.to_owned(),
            name: name.to_owned(),
            address: address.to_owned(),
        }
    }

    #[test]
    fn defaults_keep_rmcl_state_out_of_hmcl_configuration() {
        let config = default_config_path().unwrap();
        assert_eq!(config.file_name().unwrap(), "config.toml");
        assert_eq!(config.parent().unwrap().file_name().unwrap(), "rmcl");
        assert!(!config.to_string_lossy().to_lowercase().contains("hmcl"));
        assert!(default_java_store().unwrap().ends_with(".rmcl/java"));
        assert_eq!(
            LauncherConfig::default().schema_version,
            CONFIG_SCHEMA_VERSION
        );
    }

    #[test]
    fn defaults_use_platform_directories() {
        assert_eq!(home_directory(), dirs::home_dir());
        if let Some(config) = dirs::config_dir() {
            assert_eq!(default_config_dir().unwrap(), config.join("rmcl"));
        }

        let minecraft = default_minecraft_root().unwrap();
        #[cfg(windows)]
        assert_eq!(minecraft, dirs::data_dir().unwrap().join(".minecraft"));
        #[cfg(target_os = "macos")]
        assert_eq!(minecraft, dirs::data_dir().unwrap().join("minecraft"));
        #[cfg(all(unix, not(target_os = "macos")))]
        assert_eq!(minecraft, dirs::home_dir().unwrap().join(".minecraft"));
    }

    #[test]
    fn missing_file_returns_defaults() {
        let temporary = TempDir::new().unwrap();
        let store = ConfigStore::new(temporary.path().join("config.toml"));

        assert_eq!(store.load().unwrap(), LauncherConfig::default());
    }

    #[test]
    fn round_trips_accounts_and_appearance() {
        let temporary = TempDir::new().unwrap();
        let store = ConfigStore::new(temporary.path().join("rmcl/config.toml"));
        let mut config = LauncherConfig {
            selected_version: Some("1.21.1".to_owned()),
            selected_account: Some("HowDoesAuthWork".to_owned()),
            selected_account_kind: StoredAccountKind::Microsoft,
            accounts: vec![account("Steve", "5627dd98-e6be-3c21-b8a8-e92344183641")],
            microsoft_accounts: vec![microsoft_account(
                "HowDoesAuthWork",
                "986dec87-b7ec-47ff-89ff-033fdb95c4b5",
            )],
            servers: vec![server("server-1", "生存服", "play.example.com")],
            auto_install_java: false,
            auto_check_updates: false,
            download_source: DownloadSourcePreference::Mirror,
            launcher_visibility: LauncherVisibility::HideWhileRunning,
            agent_provider: "anthropic".to_owned(),
            agent_model: "claude-sonnet-5-5".to_owned(),
            agent_base_url: Some("https://gateway.example/anthropic".to_owned()),
            agent_proxy: Some("socks5://127.0.0.1:7890".to_owned()),
            ..LauncherConfig::default()
        };
        config.appearance.theme = ThemePreference::Dark;
        config.appearance.accent = AccentColor::Ocean;

        store.save(&config).unwrap();

        assert_eq!(store.load().unwrap(), config);
        assert!(store.path().exists());
        assert!(store.logs_dir().ends_with("rmcl/logs"));
    }

    #[test]
    fn schema_one_is_upgraded_with_offline_account_defaults() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("config.toml");
        let store = ConfigStore::new(&path);
        fs::write(
            &path,
            br#"schema_version = 1
minecraft_root = "/tmp/minecraft"
java_store = "/tmp/java"
selected_version = "1.21.1"
selected_account = "Steve"
auto_install_java = true
launcher_visibility = "keep_visible"

[[accounts]]
username = "Steve"
uuid = "5627dd98-e6be-3c21-b8a8-e92344183641"

[appearance]
theme = "system"
accent = "emerald"
"#,
        )
        .unwrap();

        let config = store.load().unwrap();

        assert_eq!(config.schema_version, CONFIG_SCHEMA_VERSION);
        assert_eq!(config.selected_account_kind, StoredAccountKind::Offline);
        assert_eq!(config.accounts.len(), 1);
        assert_eq!(config.accounts[0].username, "Steve");
        assert!(config.microsoft_accounts.is_empty());
    }

    #[test]
    fn schema_two_is_upgraded_with_empty_servers() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("config.toml");
        let store = ConfigStore::new(&path);
        fs::write(
            &path,
            br#"schema_version = 2
minecraft_root = "/tmp/minecraft"
java_store = "/tmp/java"
auto_install_java = true
launcher_visibility = "keep_visible"
"#,
        )
        .unwrap();

        let config = store.load().unwrap();

        assert_eq!(config.schema_version, CONFIG_SCHEMA_VERSION);
        assert!(config.servers.is_empty());
    }

    #[test]
    fn schema_three_is_upgraded_with_update_checks_enabled() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("config.toml");
        let store = ConfigStore::new(&path);
        fs::write(
            &path,
            br#"schema_version = 3
minecraft_root = "/tmp/minecraft"
java_store = "/tmp/java"
auto_install_java = true
launcher_visibility = "keep_visible"
"#,
        )
        .unwrap();

        let config = store.load().unwrap();

        assert_eq!(config.schema_version, CONFIG_SCHEMA_VERSION);
        assert!(config.auto_check_updates);
    }

    #[test]
    fn schema_four_is_upgraded_with_automatic_download_source() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("config.toml");
        let store = ConfigStore::new(&path);
        fs::write(
            &path,
            br#"schema_version = 4
minecraft_root = "/tmp/minecraft"
java_store = "/tmp/java"
auto_install_java = true
auto_check_updates = true
launcher_visibility = "keep_visible"
"#,
        )
        .unwrap();

        let config = store.load().unwrap();

        assert_eq!(config.schema_version, CONFIG_SCHEMA_VERSION);
        assert_eq!(config.download_source, DownloadSourcePreference::Auto);
    }

    #[test]
    fn schema_six_is_upgraded_with_agent_defaults() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("config.toml");
        let store = ConfigStore::new(&path);
        fs::write(
            &path,
            br#"schema_version = 6
minecraft_root = "/tmp/minecraft"
java_store = "/tmp/java"
"#,
        )
        .unwrap();

        let config = store.load().unwrap();

        assert_eq!(config.schema_version, CONFIG_SCHEMA_VERSION);
        assert_eq!(config.agent_provider, "kilo");
        assert!(config.agent_model.is_empty());
        assert_eq!(config.agent_base_url, None);
        assert_eq!(config.agent_proxy, None);
    }

    #[test]
    fn malformed_configuration_is_reported_and_preserved() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("config.toml");
        let store = ConfigStore::new(&path);
        fs::write(&path, b"schema_version = ").unwrap();

        assert!(matches!(store.load(), Err(StoreError::Toml { .. })));
        assert_eq!(fs::read(&path).unwrap(), b"schema_version = ");
    }

    #[test]
    fn newer_schema_is_rejected_without_rewriting() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("config.toml");
        let store = ConfigStore::new(&path);
        fs::write(&path, b"schema_version = 99\n").unwrap();

        assert!(matches!(
            store.load(),
            Err(StoreError::UnsupportedSchema {
                found: 99,
                supported: CONFIG_SCHEMA_VERSION
            })
        ));
        assert!(!fs::read_to_string(&path).unwrap().is_empty());
    }

    #[test]
    fn empty_schema_is_rejected() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("config.toml");
        let store = ConfigStore::new(&path);
        fs::write(&path, b"schema_version = 0\n").unwrap();

        assert!(matches!(
            store.load(),
            Err(StoreError::UnsupportedSchema { found: 0, .. })
        ));
    }

    #[test]
    fn save_overwrites_a_previous_file() {
        let temporary = TempDir::new().unwrap();
        let store = ConfigStore::new(temporary.path().join("config.toml"));
        let first = LauncherConfig {
            selected_version: Some("first".to_owned()),
            ..LauncherConfig::default()
        };
        store.save(&first).unwrap();

        let second = LauncherConfig {
            selected_version: Some("second".to_owned()),
            ..LauncherConfig::default()
        };
        store.save(&second).unwrap();

        assert_eq!(
            store.load().unwrap().selected_version.as_deref(),
            Some("second")
        );
    }
}
