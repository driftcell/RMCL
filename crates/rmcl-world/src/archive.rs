use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use rmcl_archive::{ArchiveExtractor, ExtractionLimits};
use rmcl_domain::ExtractionPlan;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use walkdir::WalkDir;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use crate::{
    CopyWorldOptions, ImportWorldOptions, World, WorldBackup, WorldCopyProgress, WorldError,
    WorldId, copy::copy_directory_to_world, lock::try_lock_world, scan::relative_time,
};

const WORLD_ARCHIVE_LIMITS: ExtractionLimits = ExtractionLimits {
    max_entries: 2_000_000,
    max_entry_size: 64 * 1024 * 1024 * 1024,
    max_total_size: 256 * 1024 * 1024 * 1024,
};

/// Imports a world folder or ZIP into `target_saves`.
pub fn import_world_zip(
    archive: &Path,
    target_saves: &Path,
    options: &ImportWorldOptions,
    cancellation: &CancellationToken,
    mut progress: impl FnMut(WorldCopyProgress),
) -> Result<World, WorldError> {
    fs::create_dir_all(target_saves).map_err(|source| WorldError::io(target_saves, source))?;
    let temporary = tempfile::Builder::new()
        .prefix(".rmcl-world-import-")
        .tempdir_in(target_saves)
        .map_err(|source| WorldError::io(target_saves, source))?;
    let extracted = temporary.path().join("extracted");
    let plan = ExtractionPlan {
        source: archive.to_path_buf(),
        destination: extracted.clone(),
        exclude_prefixes: Vec::new(),
    };
    ArchiveExtractor::new(WORLD_ARCHIVE_LIMITS).extract_zip(&plan, cancellation.clone(), None)?;
    let root = find_world_root(&extracted)?;
    let source_id = archive_world_id(archive, &root, &extracted)?;
    let copy_options = CopyWorldOptions {
        target_name: options.target_name.clone(),
        conflict: options.conflict,
        strategy: options.strategy,
    };
    let outcome = copy_directory_to_world(
        &root,
        &source_id,
        target_saves,
        &copy_options,
        cancellation,
        &mut progress,
    )?;
    Ok(outcome.world)
}

/// Exports one world as a ZIP with `session.lock` excluded.
pub fn export_world_zip(
    world: &World,
    destination: &Path,
    cancellation: &CancellationToken,
    mut progress: impl FnMut(WorldCopyProgress),
) -> Result<(), WorldError> {
    let Some(_lock) = try_lock_world(&world.path)? else {
        return Err(WorldError::WorldLocked(world.folder_name().to_owned()));
    };
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| WorldError::io(parent, source))?;
    let temp_path = parent.join(format!(".rmcl-world-export-{}.zip", Uuid::new_v4()));
    let result = write_world_zip(world, &temp_path, cancellation, &mut progress);
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
        return result;
    }
    if destination.exists() {
        fs::remove_file(destination).map_err(|source| WorldError::io(destination, source))?;
    }
    fs::rename(&temp_path, destination).map_err(|source| WorldError::io(destination, source))
}

/// Zips a world below `backups_dir` and returns the created archive.
pub fn backup_world(
    world: &World,
    backups_dir: &Path,
    cancellation: &CancellationToken,
    progress: impl FnMut(WorldCopyProgress),
) -> Result<WorldBackup, WorldError> {
    fs::create_dir_all(backups_dir).map_err(|source| WorldError::io(backups_dir, source))?;
    let created_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default();
    let mut path = backups_dir.join(format!("{created_at_ms}_{}.zip", world.id));
    for index in 1..=9999 {
        if !path.exists() {
            break;
        }
        path = backups_dir.join(format!("{created_at_ms}_{} ({index}).zip", world.id));
    }
    export_world_zip(world, &path, cancellation, progress)?;
    Ok(WorldBackup {
        path,
        world_id: world.id.clone(),
        created_at_ms,
    })
}

pub fn restore_world_backup(
    backup: &Path,
    target_saves: &Path,
    options: &ImportWorldOptions,
    cancellation: &CancellationToken,
    progress: impl FnMut(WorldCopyProgress),
) -> Result<World, WorldError> {
    import_world_zip(backup, target_saves, options, cancellation, progress)
}

/// Deletes a world directory after acquiring its `session.lock`.
pub fn delete_world(world: &World) -> Result<(), WorldError> {
    let Some(lock) = try_lock_world(&world.path)? else {
        return Err(WorldError::WorldLocked(world.folder_name().to_owned()));
    };
    // Windows cannot remove a directory while session.lock is open.
    drop(lock);
    fs::remove_dir_all(&world.path).map_err(|source| WorldError::io(&world.path, source))
}

fn write_world_zip(
    world: &World,
    destination: &Path,
    cancellation: &CancellationToken,
    progress: &mut dyn FnMut(WorldCopyProgress),
) -> Result<(), WorldError> {
    let file = File::create(destination).map_err(|source| WorldError::io(destination, source))?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let total_files = WalkDir::new(&world.path)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.file_name() != std::ffi::OsStr::new("session.lock"))
        .count();
    let mut completed_files = 0usize;
    let mut copied_bytes = 0u64;

    for entry in WalkDir::new(&world.path).follow_links(false).min_depth(1) {
        if cancellation.is_cancelled() {
            return Err(WorldError::Cancelled);
        }
        let entry = entry.map_err(|error| WorldError::io(&world.path, error.into()))?;
        let relative = entry
            .path()
            .strip_prefix(&world.path)
            .map_err(|error| WorldError::invalid_path(&world.path, error))?;
        if entry.file_type().is_symlink() {
            return Err(WorldError::SymlinkNotAllowed {
                path: entry.path().to_path_buf(),
            });
        }
        if entry.file_name() == std::ffi::OsStr::new("session.lock") {
            continue;
        }
        let name = format!(
            "{}/{}",
            world.id,
            relative.to_string_lossy().replace('\\', "/")
        );
        if entry.file_type().is_dir() {
            zip.add_directory(format!("{name}/"), options)
                .map_err(|source| WorldError::ZipWrite {
                    path: destination.to_path_buf(),
                    source,
                })?;
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        zip.start_file(name, options)
            .map_err(|source| WorldError::ZipWrite {
                path: destination.to_path_buf(),
                source,
            })?;
        let bytes = std::io::copy(
            &mut File::open(entry.path()).map_err(|source| WorldError::io(entry.path(), source))?,
            &mut zip,
        )
        .map_err(|source| WorldError::io(destination, source))?;
        completed_files += 1;
        copied_bytes = copied_bytes.saturating_add(bytes);
        progress(WorldCopyProgress {
            current: relative.to_path_buf(),
            completed_files,
            total_files,
            copied_bytes,
            total_bytes: world.size_bytes,
            cloned_files: 0,
        });
    }
    let file = zip.finish().map_err(|source| WorldError::ZipWrite {
        path: destination.to_path_buf(),
        source,
    })?;
    file.sync_all()
        .map_err(|source| WorldError::io(destination, source))?;
    Ok(())
}

fn find_world_root(extracted: &Path) -> Result<PathBuf, WorldError> {
    if extracted.join("level.dat").is_file() {
        return Ok(extracted.to_path_buf());
    }
    let mut worlds = Vec::new();
    let entries = fs::read_dir(extracted).map_err(|source| WorldError::io(extracted, source))?;
    for entry in entries {
        let entry = entry.map_err(|source| WorldError::io(extracted, source))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| WorldError::io(&path, source))?;
        if file_type.is_dir()
            && entry.file_name() != std::ffi::OsStr::new("__MACOSX")
            && path.join("level.dat").is_file()
        {
            worlds.push(path);
        }
    }
    match worlds.as_slice() {
        [world] => Ok(world.clone()),
        [] => Err(WorldError::InvalidArchive),
        _ => Err(WorldError::MultipleWorlds),
    }
}

fn archive_world_id(archive: &Path, root: &Path, extracted: &Path) -> Result<WorldId, WorldError> {
    if root != extracted
        && let Some(name) = root.file_name().and_then(|name| name.to_str())
        && let Ok(id) = WorldId::new(name)
    {
        return Ok(id);
    }
    let stem = archive
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("world");
    WorldId::new(stem).or_else(|_| WorldId::new("world"))
}

/// Human-readable backup label used by the UI.
pub fn backup_label(backup: &WorldBackup) -> String {
    relative_time(Some(backup.created_at_ms as i64)).unwrap_or_else(|| backup.world_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_archive_limits_accept_large_worlds() {
        const { assert!(WORLD_ARCHIVE_LIMITS.max_total_size > 8 * 1024 * 1024 * 1024) };
    }
}
