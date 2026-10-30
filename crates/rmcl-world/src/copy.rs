use std::{
    fs,
    path::{Path, PathBuf},
};

use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::{
    CopyStrategy, CopyWorldOptions, CopyWorldOutcome, World, WorldConflictPolicy,
    WorldCopyProgress, WorldError, WorldId, lock::try_lock_world, scan::read_world,
};

#[derive(Debug, Clone, Copy)]
struct TreeStats {
    files: usize,
    bytes: u64,
}

#[derive(Debug, Clone, Copy)]
struct CopyStats {
    files: usize,
    cloned_files: usize,
    bytes: u64,
}

/// Copies one world into `target_saves`.
///
/// The source is locked for the duration, `session.lock` is never copied, and
/// the destination is published with a staged rename. Symbolic links inside
/// the world are rejected instead of followed.
pub fn copy_world(
    source: &World,
    target_saves: &Path,
    options: &CopyWorldOptions,
    cancellation: &CancellationToken,
    mut progress: impl FnMut(WorldCopyProgress),
) -> Result<CopyWorldOutcome, WorldError> {
    validate_world_directory(&source.path)?;
    let Some(_lock) = try_lock_world(&source.path)? else {
        return Err(WorldError::WorldLocked(source.folder_name().to_owned()));
    };
    copy_directory_to_world(
        &source.path,
        &source.id,
        target_saves,
        options,
        cancellation,
        &mut progress,
    )
}

/// Moves one world into `target_saves`.
///
/// Same-filesystem moves use an atomic rename. A cross-filesystem move falls
/// back to a staged copy, then removes the source only after the target is
/// complete.
pub fn move_world(
    source: &World,
    target_saves: &Path,
    options: &CopyWorldOptions,
    cancellation: &CancellationToken,
    mut progress: impl FnMut(WorldCopyProgress),
) -> Result<CopyWorldOutcome, WorldError> {
    validate_world_directory(&source.path)?;
    let Some(lock) = try_lock_world(&source.path)? else {
        return Err(WorldError::WorldLocked(source.folder_name().to_owned()));
    };
    fs::create_dir_all(target_saves).map_err(|source| WorldError::io(target_saves, source))?;
    let target_name = target_name(&source.id, options)?;
    let target_path = resolve_target_path(target_saves, &target_name, options.conflict)?;
    if target_path == source.path {
        drop(lock);
        return Ok(CopyWorldOutcome {
            world: source.clone(),
            target_path,
            copied_files: 0,
            cloned_files: 0,
            copied_bytes: 0,
        });
    }

    // A directory rename fails on Windows while session.lock is open.
    drop(lock);
    let renamed = match options.conflict {
        WorldConflictPolicy::Replace if target_path.exists() => {
            replace_with_rename(&source.path, &target_path)
        }
        _ => fs::rename(&source.path, &target_path)
            .map_err(|source_error| WorldError::io(&source.path, source_error)),
    };
    match renamed {
        Ok(()) => Ok(CopyWorldOutcome {
            world: read_world(&target_path)?,
            target_path,
            copied_files: 0,
            cloned_files: 0,
            copied_bytes: 0,
        }),
        Err(error) if is_cross_device(&error) => {
            let Some(lock) = try_lock_world(&source.path)? else {
                return Err(WorldError::WorldLocked(source.folder_name().to_owned()));
            };
            let outcome = copy_directory_to_world(
                &source.path,
                &source.id,
                target_saves,
                options,
                cancellation,
                &mut progress,
            )?;
            drop(lock);
            fs::remove_dir_all(&source.path)
                .map_err(|source_error| WorldError::io(&source.path, source_error))?;
            Ok(outcome)
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn copy_directory_to_world(
    source_dir: &Path,
    source_id: &WorldId,
    target_saves: &Path,
    options: &CopyWorldOptions,
    cancellation: &CancellationToken,
    progress: &mut dyn FnMut(WorldCopyProgress),
) -> Result<CopyWorldOutcome, WorldError> {
    if cancellation.is_cancelled() {
        return Err(WorldError::Cancelled);
    }
    fs::create_dir_all(target_saves).map_err(|source| WorldError::io(target_saves, source))?;
    let target_name = target_name(source_id, options)?;
    let final_path = resolve_target_path(target_saves, &target_name, options.conflict)?;
    let staging = target_saves.join(format!(".rmcl-world-{}", Uuid::new_v4()));
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|source| WorldError::io(&staging, source))?;
    }
    fs::create_dir(&staging).map_err(|source| WorldError::io(&staging, source))?;

    let stats = collect_stats(source_dir, cancellation)?;
    let result = (|| {
        let copied = copy_tree(
            source_dir,
            &staging,
            options.strategy,
            stats,
            cancellation,
            progress,
        )?;
        if cancellation.is_cancelled() {
            return Err(WorldError::Cancelled);
        }
        publish_staging(&staging, &final_path, options.conflict)?;
        Ok((read_world(&final_path)?, copied))
    })();

    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    let (world, copied) = result?;
    Ok(CopyWorldOutcome {
        world,
        target_path: final_path,
        copied_files: copied.files,
        cloned_files: copied.cloned_files,
        copied_bytes: copied.bytes,
    })
}

fn target_name(source_id: &WorldId, options: &CopyWorldOptions) -> Result<WorldId, WorldError> {
    match options.target_name.as_deref() {
        Some(name) => WorldId::new(name),
        None => Ok(source_id.clone()),
    }
}

fn resolve_target_path(
    saves_dir: &Path,
    name: &WorldId,
    conflict: WorldConflictPolicy,
) -> Result<PathBuf, WorldError> {
    let candidate = saves_dir.join(name.as_str());
    if !candidate.exists() {
        return Ok(candidate);
    }
    match conflict {
        WorldConflictPolicy::Rename => {
            for index in 1..=9999 {
                let candidate = saves_dir.join(format!("{} ({index})", name.as_str()));
                if !candidate.exists() {
                    return Ok(candidate);
                }
            }
            Err(WorldError::WorldExists(name.to_string()))
        }
        WorldConflictPolicy::Reject => Err(WorldError::WorldExists(name.to_string())),
        WorldConflictPolicy::Replace => Ok(candidate),
    }
}

fn publish_staging(
    staging: &Path,
    final_path: &Path,
    conflict: WorldConflictPolicy,
) -> Result<(), WorldError> {
    if !final_path.exists() {
        return fs::rename(staging, final_path)
            .map_err(|source| WorldError::io(final_path, source));
    }
    if conflict != WorldConflictPolicy::Replace {
        return Err(WorldError::WorldExists(
            final_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        ));
    }
    let old_path = final_path.with_file_name(format!(".rmcl-old-{}", Uuid::new_v4()));
    fs::rename(final_path, &old_path).map_err(|source| WorldError::io(final_path, source))?;
    match fs::rename(staging, final_path) {
        Ok(()) => {
            let _ = fs::remove_dir_all(&old_path);
            Ok(())
        }
        Err(source) => {
            let _ = fs::rename(&old_path, final_path);
            Err(WorldError::io(final_path, source))
        }
    }
}

fn replace_with_rename(source: &Path, target: &Path) -> Result<(), WorldError> {
    let old_path = target.with_file_name(format!(".rmcl-old-{}", Uuid::new_v4()));
    fs::rename(target, &old_path).map_err(|error| WorldError::io(target, error))?;
    match fs::rename(source, target) {
        Ok(()) => {
            let _ = fs::remove_dir_all(&old_path);
            Ok(())
        }
        Err(error) => {
            let _ = fs::rename(&old_path, target);
            Err(WorldError::io(source, error))
        }
    }
}

fn copy_tree(
    source_dir: &Path,
    target_dir: &Path,
    strategy: CopyStrategy,
    stats: TreeStats,
    cancellation: &CancellationToken,
    progress: &mut dyn FnMut(WorldCopyProgress),
) -> Result<CopyStats, WorldError> {
    let mut completed_files = 0usize;
    let mut copied_bytes = 0u64;
    let mut cloned_files = 0usize;

    for entry in WalkDir::new(source_dir).follow_links(false).min_depth(1) {
        if cancellation.is_cancelled() {
            return Err(WorldError::Cancelled);
        }
        let entry = entry.map_err(|error| WorldError::io(source_dir, error.into()))?;
        let relative = entry
            .path()
            .strip_prefix(source_dir)
            .map_err(|error| WorldError::invalid_path(source_dir, error))?;
        let target = target_dir.join(relative);
        let file_type = entry.file_type();
        if file_type.is_symlink() {
            return Err(WorldError::SymlinkNotAllowed {
                path: entry.path().to_path_buf(),
            });
        }
        if file_type.is_dir() {
            fs::create_dir_all(&target).map_err(|source| WorldError::io(&target, source))?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        if relative == Path::new("session.lock") {
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|source| WorldError::io(parent, source))?;
        }
        let copied = copy_file(entry.path(), &target, strategy)?;
        if copied.cloned {
            cloned_files += 1;
        }
        completed_files += 1;
        copied_bytes = copied_bytes.saturating_add(copied.bytes);
        progress(WorldCopyProgress {
            current: relative.to_path_buf(),
            completed_files,
            total_files: stats.files,
            copied_bytes,
            total_bytes: stats.bytes,
            cloned_files,
        });
    }
    Ok(CopyStats {
        files: completed_files,
        cloned_files,
        bytes: copied_bytes,
    })
}

struct CopiedFile {
    bytes: u64,
    cloned: bool,
}

fn copy_file(
    source: &Path,
    target: &Path,
    strategy: CopyStrategy,
) -> Result<CopiedFile, WorldError> {
    let metadata = fs::metadata(source).map_err(|error| WorldError::io(source, error))?;
    let result = match strategy {
        CopyStrategy::Auto => match reflink_copy::reflink_or_copy(source, target) {
            Ok(None) => Ok(CopiedFile {
                bytes: metadata.len(),
                cloned: true,
            }),
            Ok(Some(bytes)) => Ok(CopiedFile {
                bytes,
                cloned: false,
            }),
            Err(error) => Err(WorldError::io(target, error)),
        },
        CopyStrategy::Copy => fs::copy(source, target)
            .map(|bytes| CopiedFile {
                bytes,
                cloned: false,
            })
            .map_err(|error| WorldError::io(target, error)),
    }?;
    let _ = fs::set_permissions(target, metadata.permissions());
    Ok(result)
}

fn collect_stats(
    source_dir: &Path,
    cancellation: &CancellationToken,
) -> Result<TreeStats, WorldError> {
    let mut files = 0usize;
    let mut bytes = 0u64;
    for entry in WalkDir::new(source_dir).follow_links(false).min_depth(1) {
        if cancellation.is_cancelled() {
            return Err(WorldError::Cancelled);
        }
        let entry = entry.map_err(|error| WorldError::io(source_dir, error.into()))?;
        if entry.file_type().is_symlink() {
            return Err(WorldError::SymlinkNotAllowed {
                path: entry.path().to_path_buf(),
            });
        }
        if entry.file_type().is_file() && entry.file_name() != std::ffi::OsStr::new("session.lock")
        {
            files += 1;
            bytes =
                bytes.saturating_add(entry.metadata().map(|metadata| metadata.len()).unwrap_or(0));
        }
    }
    Ok(TreeStats { files, bytes })
}

fn validate_world_directory(path: &Path) -> Result<(), WorldError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| WorldError::io(path, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(WorldError::InvalidWorld {
            path: path.to_path_buf(),
        });
    }
    let level_dat = path.join("level.dat");
    if !level_dat.is_file() {
        return Err(WorldError::InvalidWorld {
            path: path.to_path_buf(),
        });
    }
    if let Ok(metadata) = fs::symlink_metadata(path.join("session.lock"))
        && metadata.file_type().is_symlink()
    {
        return Err(WorldError::SymlinkNotAllowed {
            path: path.join("session.lock"),
        });
    }
    Ok(())
}

fn is_cross_device(error: &WorldError) -> bool {
    let WorldError::Io { source, .. } = error else {
        return false;
    };
    source.kind() == std::io::ErrorKind::CrossesDevices || source.raw_os_error() == Some(18)
}
