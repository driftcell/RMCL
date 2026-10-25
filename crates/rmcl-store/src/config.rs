use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::StoreError;

pub const CONFIG_SCHEMA_VERSION: u32 = 1;

/// Launcher-owned settings. This file never replaces or rewrites HMCL configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LauncherConfig {
    pub schema_version: u32,
    pub minecraft_root: PathBuf,
    pub java_store: PathBuf,
    pub selected_version: Option<String>,
    pub selected_account: Option<String>,
    pub accounts: Vec<StoredOfflineAccount>,
    pub appearance: AppearanceConfig,
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            minecraft_root: default_minecraft_root(),
            java_store: default_java_store(),
            selected_version: None,
            selected_account: None,
            accounts: Vec::new(),
            appearance: AppearanceConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredOfflineAccount {
    pub username: String,
    pub uuid: String,
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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

    pub fn discover() -> Self {
        Self::new(default_config_path())
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
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
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
        let config: LauncherConfig =
            serde_json::from_slice(&bytes).map_err(|source| StoreError::Json {
                path: self.path.clone(),
                source,
            })?;
        if config.schema_version == 0 || config.schema_version > CONFIG_SCHEMA_VERSION {
            return Err(StoreError::UnsupportedSchema {
                found: config.schema_version,
                supported: CONFIG_SCHEMA_VERSION,
            });
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
        let mut temporary = NamedTempFile::new_in(parent).map_err(|source| StoreError::Io {
            operation: "create configuration temporary file",
            path: parent.to_path_buf(),
            source,
        })?;
        serde_json::to_writer_pretty(&mut temporary, config).map_err(|source| {
            StoreError::Json {
                path: self.path.clone(),
                source,
            }
        })?;
        temporary
            .write_all(b"\n")
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

pub fn home_directory() -> PathBuf {
    #[cfg(windows)]
    let variable = "USERPROFILE";
    #[cfg(not(windows))]
    let variable = "HOME";
    std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub fn default_minecraft_root() -> PathBuf {
    home_directory().join(".minecraft")
}

pub fn default_java_store() -> PathBuf {
    home_directory().join(".rmcl").join("java")
}

pub fn default_config_path() -> PathBuf {
    home_directory().join(".rmcl").join("config.json")
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

    #[test]
    fn defaults_keep_rmcl_state_out_of_hmcl_configuration() {
        assert!(default_config_path().ends_with(".rmcl/config.json"));
        assert!(default_minecraft_root().ends_with(".minecraft"));
        assert!(default_java_store().ends_with(".rmcl/java"));
        assert_eq!(
            LauncherConfig::default().schema_version,
            CONFIG_SCHEMA_VERSION
        );
    }

    #[test]
    fn missing_file_returns_defaults() {
        let temporary = TempDir::new().unwrap();
        let store = ConfigStore::new(temporary.path().join("config.json"));

        assert_eq!(store.load().unwrap(), LauncherConfig::default());
    }

    #[test]
    fn round_trips_accounts_and_appearance() {
        let temporary = TempDir::new().unwrap();
        let store = ConfigStore::new(temporary.path().join("rmcl/config.json"));
        let mut config = LauncherConfig {
            selected_version: Some("1.21.1".to_owned()),
            selected_account: Some("Steve".to_owned()),
            accounts: vec![account("Steve", "5627dd98-e6be-3c21-b8a8-e92344183641")],
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
    fn malformed_configuration_is_reported_and_preserved() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("config.json");
        let store = ConfigStore::new(&path);
        fs::write(&path, b"{ not json").unwrap();

        assert!(matches!(store.load(), Err(StoreError::Json { .. })));
        assert_eq!(fs::read(&path).unwrap(), b"{ not json");
    }

    #[test]
    fn newer_schema_is_rejected_without_rewriting() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("config.json");
        let store = ConfigStore::new(&path);
        fs::write(
            &path,
            br#"{"schema_version": 99, "accounts": [{"username": "Steve", "uuid": "x"}]}"#,
        )
        .unwrap();

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
        let path = temporary.path().join("config.json");
        let store = ConfigStore::new(&path);
        fs::write(&path, br#"{"schema_version": 0}"#).unwrap();

        assert!(matches!(
            store.load(),
            Err(StoreError::UnsupportedSchema { found: 0, .. })
        ));
    }

    #[test]
    fn save_overwrites_a_previous_file() {
        let temporary = TempDir::new().unwrap();
        let store = ConfigStore::new(temporary.path().join("config.json"));
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
