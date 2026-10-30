use std::{
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use walkdir::WalkDir;

use crate::{
    World, WorldError, WorldId, WorldScan, WorldWarning,
    lock::is_world_locked,
    nbt::{LevelMetadata, read_level_dat},
};

const MAX_ICON_BYTES: u64 = 4 * 1024 * 1024;

/// Reads one world directory below `saves/`.
pub fn read_world(world_dir: impl AsRef<Path>) -> Result<World, WorldError> {
    let world_dir = world_dir.as_ref();
    let metadata =
        std::fs::symlink_metadata(world_dir).map_err(|source| WorldError::io(world_dir, source))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(WorldError::InvalidWorld {
            path: world_dir.to_path_buf(),
        });
    }
    let folder_name = world_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| WorldError::InvalidWorld {
            path: world_dir.to_path_buf(),
        })?;
    let id = WorldId::new(folder_name)?;
    let level_dat = world_dir.join("level.dat");
    if !level_dat.is_file() {
        return Err(WorldError::InvalidWorld {
            path: world_dir.to_path_buf(),
        });
    }
    let level = read_level_dat(&level_dat)?;
    let icon_path = usable_icon(world_dir);
    Ok(world_from_level(id, world_dir, level, icon_path))
}

/// Scans `saves/` without following symlinked world directories.
pub fn scan_worlds(saves_dir: impl AsRef<Path>) -> Result<WorldScan, WorldError> {
    let saves_dir = saves_dir.as_ref();
    let entries = match std::fs::read_dir(saves_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(WorldScan::default());
        }
        Err(source) => return Err(WorldError::io(saves_dir, source)),
    };

    let mut scan = WorldScan::default();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                scan.warnings.push(WorldWarning {
                    path: saves_dir.to_path_buf(),
                    message: error.to_string(),
                });
                continue;
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                scan.warnings.push(WorldWarning {
                    path,
                    message: error.to_string(),
                });
                continue;
            }
        };
        if file_type.is_symlink() {
            scan.warnings.push(WorldWarning {
                path,
                message: "已跳过符号链接世界".to_owned(),
            });
            continue;
        }
        if !file_type.is_dir() {
            continue;
        }
        match read_world(&path) {
            Ok(world) => scan.worlds.push(world),
            Err(WorldError::InvalidWorld { .. }) => {}
            Err(error) => scan.warnings.push(WorldWarning {
                path: path.clone(),
                message: error.to_string(),
            }),
        }
    }
    scan.worlds.sort_by(|left, right| {
        right
            .last_played
            .unwrap_or_default()
            .cmp(&left.last_played.unwrap_or_default())
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(scan)
}

fn world_from_level(
    id: WorldId,
    path: &Path,
    level: LevelMetadata,
    icon_path: Option<PathBuf>,
) -> World {
    World {
        id,
        path: path.to_path_buf(),
        name: level
            .name
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| {
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            }),
        game_version: level.game_version,
        data_version: level.data_version,
        last_played: level.last_played,
        was_modded: level.was_modded,
        enabled_features: level.enabled_features,
        enabled_datapacks: level.enabled_datapacks,
        size_bytes: directory_size(path),
        icon_path,
        locked: is_world_locked(path),
    }
}

fn usable_icon(world_dir: &Path) -> Option<PathBuf> {
    let path = world_dir.join("icon.png");
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= MAX_ICON_BYTES
    {
        Some(path)
    } else {
        None
    }
}

fn directory_size(directory: &Path) -> u64 {
    WalkDir::new(directory)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| metadata.len())
        .sum()
}

/// Formats `Data.LastPlayed` as a short relative time without a date library.
pub fn relative_time(timestamp_ms: Option<i64>) -> Option<String> {
    let timestamp_ms = timestamp_ms?;
    if timestamp_ms <= 0 {
        return None;
    }
    let now = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis() as i64;
    let seconds = (now - timestamp_ms).max(0) / 1000;
    Some(match seconds {
        0..=59 => "刚刚".to_owned(),
        60..=3_599 => format!("{} 分钟前", seconds / 60),
        3_600..=86_399 => format!("{} 小时前", seconds / 3_600),
        86_400..=2_591_999 => format!("{} 天前", seconds / 86_400),
        _ => format!("{} 个月前", seconds / 2_592_000),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_saves_directory_is_empty() {
        let temporary = tempfile::tempdir().unwrap();
        let scan = scan_worlds(temporary.path().join("saves")).unwrap();
        assert!(scan.worlds.is_empty());
        assert!(scan.warnings.is_empty());
    }

    #[test]
    fn relative_time_handles_old_and_missing_values() {
        assert!(relative_time(None).is_none());
        assert!(relative_time(Some(0)).is_none());
        assert_eq!(relative_time(Some(i64::MAX / 2)), Some("刚刚".to_owned()));
    }
}
