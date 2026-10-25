//! Launcher configuration persistence with schema validation and atomic writes.

mod config;
mod error;

pub use config::{
    AccentColor, AppearanceConfig, CONFIG_SCHEMA_VERSION, ConfigStore, LauncherConfig,
    StoredOfflineAccount, ThemePreference, default_config_path, default_java_store,
    default_minecraft_root, home_directory,
};
pub use error::StoreError;
