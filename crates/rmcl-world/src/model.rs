use std::{
    fmt,
    path::{Path, PathBuf},
};

use rmcl_domain::validate_portable_component;
use serde::{Deserialize, Serialize};

use crate::WorldError;

/// A Minecraft world directory name below `saves/`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct WorldId(String);

impl WorldId {
    pub fn new(value: impl Into<String>) -> Result<Self, WorldError> {
        let value = value.into();
        validate_portable_component(&value)
            .map_err(|_| WorldError::InvalidWorldId(value.clone()))?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WorldId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Metadata read from one world directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct World {
    pub id: WorldId,
    pub path: PathBuf,
    /// `Data.LevelName`, with section formatting codes removed for display.
    pub name: String,
    /// `Data.Version.Name`, when the world records it.
    pub game_version: Option<String>,
    /// `Data.DataVersion`, falling back to `Data.Version.Id`.
    pub data_version: Option<i32>,
    /// `Data.LastPlayed`, in Unix milliseconds.
    pub last_played: Option<i64>,
    pub was_modded: bool,
    pub enabled_features: Vec<String>,
    pub enabled_datapacks: Vec<String>,
    pub size_bytes: u64,
    pub icon_path: Option<PathBuf>,
    pub locked: bool,
}

impl World {
    pub fn folder_name(&self) -> &str {
        self.id.as_str()
    }

    pub fn compatibility(&self, target_world_version: Option<i32>) -> WorldCompatibility {
        match (self.data_version, target_world_version) {
            (Some(world), Some(target)) if world == target => WorldCompatibility::Same,
            (Some(world), Some(target)) if world < target => WorldCompatibility::Upgrade,
            (Some(_), Some(_)) => WorldCompatibility::Downgrade,
            _ => WorldCompatibility::Unknown,
        }
    }

    pub fn icon(&self) -> Option<&Path> {
        self.icon_path.as_deref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldCompatibility {
    Same,
    Upgrade,
    Downgrade,
    Unknown,
}

impl WorldCompatibility {
    pub fn label(self) -> &'static str {
        match self {
            Self::Same => "版本一致",
            Self::Upgrade => "将升级",
            Self::Downgrade => "需要更高版本",
            Self::Unknown => "版本未知",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldWarning {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorldScan {
    pub worlds: Vec<World>,
    pub warnings: Vec<WorldWarning>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorldConflictPolicy {
    /// Keep the source name, adding ` (1)`, ` (2)` when needed.
    #[default]
    Rename,
    /// Refuse to write when the target name exists.
    Reject,
    /// Replace an existing world after the caller has made a backup.
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CopyStrategy {
    /// Try a copy-on-write clone per file, then fall back to a regular copy.
    #[default]
    Auto,
    /// Always use a regular byte copy.
    Copy,
}

#[derive(Debug, Clone)]
pub struct CopyWorldOptions {
    /// `None` keeps the source folder name.
    pub target_name: Option<String>,
    pub conflict: WorldConflictPolicy,
    pub strategy: CopyStrategy,
}

impl Default for CopyWorldOptions {
    fn default() -> Self {
        Self {
            target_name: None,
            conflict: WorldConflictPolicy::Rename,
            strategy: CopyStrategy::Auto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldCopyProgress {
    pub current: PathBuf,
    pub completed_files: usize,
    pub total_files: usize,
    pub copied_bytes: u64,
    pub total_bytes: u64,
    pub cloned_files: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyWorldOutcome {
    pub world: World,
    pub target_path: PathBuf,
    pub copied_files: usize,
    pub cloned_files: usize,
    pub copied_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldBackup {
    pub path: PathBuf,
    pub world_id: WorldId,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone)]
pub struct ImportWorldOptions {
    pub target_name: Option<String>,
    pub conflict: WorldConflictPolicy,
    pub strategy: CopyStrategy,
}

impl Default for ImportWorldOptions {
    fn default() -> Self {
        Self {
            target_name: None,
            conflict: WorldConflictPolicy::Rename,
            strategy: CopyStrategy::Auto,
        }
    }
}
