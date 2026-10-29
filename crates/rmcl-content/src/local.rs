use std::{
    fs::{self, File},
    path::{Component, Path, PathBuf},
};

use rmcl_instance::{InstanceLayout, RepositoryPath};
use tempfile::NamedTempFile;

use crate::{
    ContentError, ContentKind, ContentLibrary, ContentScan, ContentWarning, LocalContent,
    kind::strip_disabled_suffix,
    options::{ResourcePackOptions, ResourcePackOrder, modern_resource_pack_list},
};

/// Policy used when an imported content file collides with an existing name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConflictPolicy {
    /// Keep the existing file and choose a new `name (1).ext` destination.
    #[default]
    Rename,
    /// Fail with [`ContentError::Conflict`].
    Reject,
}

/// Scans all content kinds of one instance.
///
/// Per-entry failures are reported as warnings instead of hiding the whole
/// library. Missing content directories are treated as empty.
pub fn scan_library(instance: &InstanceLayout) -> ContentLibrary {
    let mut library = ContentLibrary::default();
    for kind in ContentKind::ALL {
        let scan = scan_kind(instance, kind);
        library.items.extend(scan.items);
        library.warnings.extend(scan.warnings);
    }
    library.items.sort_by(|left, right| {
        left.kind.cmp(&right.kind).then_with(|| {
            left.display_name()
                .to_lowercase()
                .cmp(&right.display_name().to_lowercase())
        })
    });
    library
}

/// Scans one content kind, reporting non-fatal issues as warnings.
pub fn scan_kind(instance: &InstanceLayout, kind: ContentKind) -> ContentScan {
    let mut warnings = Vec::new();
    let items = match scan_kind_inner(instance, kind, &mut warnings) {
        Ok(items) => items,
        Err(error) => {
            warnings.push(ContentWarning::Scan {
                kind,
                message: error.to_string(),
            });
            Vec::new()
        }
    };
    ContentScan { items, warnings }
}

/// Imports one local file into an instance content directory.
///
/// The source must be a regular file accepted by `kind`. With
/// [`ConflictPolicy::Rename`], an existing name is never overwritten.
pub fn import_file(
    instance: &InstanceLayout,
    kind: ContentKind,
    source: impl AsRef<Path>,
    policy: ConflictPolicy,
) -> Result<LocalContent, ContentError> {
    let source = source.as_ref();
    let metadata = fs::symlink_metadata(source).map_err(|source_error| ContentError::Io {
        operation: "inspect imported content",
        path: source.to_path_buf(),
        source: source_error,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(ContentError::Symlink {
            path: source.to_path_buf(),
        });
    }
    if !metadata.is_file() {
        return Err(ContentError::UnsupportedFile {
            kind,
            path: source.to_path_buf(),
        });
    }
    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ContentError::UnsafePath {
            path: source.to_path_buf(),
            reason: "file name is not valid UTF-8".to_owned(),
        })?;
    if !kind.accepts_file_name(file_name) {
        return Err(ContentError::UnsupportedFile {
            kind,
            path: source.to_path_buf(),
        });
    }

    let directory = kind.directory(instance);
    ensure_content_directory(&directory)?;
    let target_name = match policy {
        ConflictPolicy::Reject => {
            if exists_case_insensitive(&directory, file_name)? {
                return Err(ContentError::Conflict {
                    path: directory.join(file_name),
                });
            }
            file_name.to_owned()
        }
        ConflictPolicy::Rename => unique_name(&directory, file_name)?,
    };
    let destination = directory.join(&target_name);

    let mut input = File::open(source).map_err(|source_error| ContentError::Io {
        operation: "open imported content",
        path: source.to_path_buf(),
        source: source_error,
    })?;
    let mut temporary =
        NamedTempFile::new_in(&directory).map_err(|source_error| ContentError::Io {
            operation: "create imported content temporary file",
            path: directory.clone(),
            source: source_error,
        })?;
    std::io::copy(&mut input, temporary.as_file_mut()).map_err(|source_error| {
        ContentError::Io {
            operation: "write imported content",
            path: temporary.path().to_path_buf(),
            source: source_error,
        }
    })?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|source_error| ContentError::Io {
            operation: "sync imported content",
            path: temporary.path().to_path_buf(),
            source: source_error,
        })?;
    temporary.persist_noclobber(&destination).map_err(|error| {
        if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            ContentError::Conflict {
                path: destination.clone(),
            }
        } else {
            ContentError::Io {
                operation: "publish imported content",
                path: destination.clone(),
                source: error.error,
            }
        }
    })?;

    let enabled = match kind {
        ContentKind::Mod => !destination
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".disabled")),
        ContentKind::ResourcePack => false,
        ContentKind::ShaderPack => true,
    };
    make_item(kind, &directory, &destination, Some(enabled))
}

/// Replaces one existing content file while preserving its relative path and
/// enable state. The old file is restored if publishing the replacement fails.
pub fn replace_content_file(
    instance: &InstanceLayout,
    kind: ContentKind,
    source: &Path,
    relative_path: &RepositoryPath,
) -> Result<LocalContent, ContentError> {
    let directory = kind.directory(instance);
    let destination = ensure_safe_existing(&directory, relative_path)?;
    let metadata = fs::symlink_metadata(&destination).map_err(|source| ContentError::Io {
        operation: "inspect replaced content",
        path: destination.clone(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(ContentError::Symlink { path: destination });
    }
    if !metadata.is_file() {
        return Err(ContentError::UnsupportedOperation {
            kind,
            operation: "replace a non-file entry",
        });
    }
    let parent = destination
        .parent()
        .ok_or_else(|| ContentError::UnsafePath {
            path: destination.clone(),
            reason: "content path has no parent".to_owned(),
        })?;
    let mut temporary = NamedTempFile::new_in(parent).map_err(|source| ContentError::Io {
        operation: "create replacement temporary file",
        path: destination.clone(),
        source,
    })?;
    let mut input = File::open(source).map_err(|error| ContentError::Io {
        operation: "open replacement source",
        path: source.to_path_buf(),
        source: error,
    })?;
    std::io::copy(&mut input, temporary.as_file_mut()).map_err(|source| ContentError::Io {
        operation: "write replacement content",
        path: temporary.path().to_path_buf(),
        source,
    })?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|source| ContentError::Io {
            operation: "sync replacement content",
            path: temporary.path().to_path_buf(),
            source,
        })?;

    let backup = NamedTempFile::new_in(parent).map_err(|source| ContentError::Io {
        operation: "reserve replacement backup",
        path: destination.clone(),
        source,
    })?;
    let backup_path = backup.into_temp_path();
    fs::remove_file(&backup_path).map_err(|source| ContentError::Io {
        operation: "prepare replacement backup",
        path: backup_path.to_path_buf(),
        source,
    })?;
    fs::rename(&destination, &backup_path).map_err(|source| ContentError::Io {
        operation: "backup replaced content",
        path: destination.clone(),
        source,
    })?;
    if let Err(error) = temporary.persist_noclobber(&destination) {
        let restore = fs::rename(&backup_path, &destination);
        return match restore {
            Ok(()) => Err(ContentError::Io {
                operation: "publish replacement content",
                path: destination,
                source: error.error,
            }),
            Err(source) => Err(ContentError::Io {
                operation: "restore content after replacement failure",
                path: backup_path.to_path_buf(),
                source,
            }),
        };
    }
    drop(backup_path);
    make_item(kind, &directory, &destination, None)
}

/// Deletes one content entry and updates resource-pack options when needed.
pub fn delete_content(
    instance: &InstanceLayout,
    kind: ContentKind,
    relative_path: &RepositoryPath,
) -> Result<(), ContentError> {
    let directory = kind.directory(instance);
    let path = ensure_safe_existing(&directory, relative_path)?;
    let metadata = fs::symlink_metadata(&path).map_err(|source| ContentError::Io {
        operation: "inspect deleted content",
        path: path.clone(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(ContentError::Symlink { path });
    }
    if metadata.is_dir() && !kind.accepts_directory() {
        return Err(ContentError::UnsupportedOperation {
            kind,
            operation: "delete a directory",
        });
    }

    if kind == ContentKind::ResourcePack {
        let file_name = file_name(relative_path);
        let options_path = instance.options_path();
        if let Some(mut options) =
            ResourcePackOptions::load(options_path.clone(), modern_hint(instance))?
            && options.contains(file_name)
        {
            options.set_enabled(file_name, false);
            options.save()?;
        }
    }

    if metadata.is_dir() {
        fs::remove_dir_all(&path).map_err(|source| ContentError::Io {
            operation: "delete content directory",
            path,
            source,
        })
    } else {
        fs::remove_file(&path).map_err(|source| ContentError::Io {
            operation: "delete content file",
            path,
            source,
        })
    }
}

/// Enables or disables a mod or resource pack.
///
/// Mods are toggled with the conventional `.disabled` suffix. Resource packs
/// are toggled by rewriting the instance's `options.txt` lists atomically.
/// Shader packs have no standard enable flag and return an error.
pub fn set_content_enabled(
    instance: &InstanceLayout,
    kind: ContentKind,
    relative_path: &RepositoryPath,
    enabled: bool,
) -> Result<LocalContent, ContentError> {
    let directory = kind.directory(instance);
    let path = ensure_safe_existing(&directory, relative_path)?;
    match kind {
        ContentKind::Mod => toggle_mod(&directory, &path, enabled),
        ContentKind::ResourcePack => {
            let file_name = file_name(relative_path);
            let options_path = instance.options_path();
            let modern = modern_hint(instance);
            let mut options = ResourcePackOptions::load(options_path.clone(), modern)?
                .unwrap_or_else(|| ResourcePackOptions::new(options_path, modern));
            options.set_enabled(file_name, enabled);
            options.save()?;
            make_item(kind, &directory, &path, Some(enabled))
        }
        ContentKind::ShaderPack => Err(ContentError::UnsupportedOperation {
            kind,
            operation: "toggle content",
        }),
    }
}

fn scan_kind_inner(
    instance: &InstanceLayout,
    kind: ContentKind,
    warnings: &mut Vec<ContentWarning>,
) -> Result<Vec<LocalContent>, ContentError> {
    match kind {
        ContentKind::Mod => scan_mods(instance, warnings),
        ContentKind::ResourcePack => scan_resource_packs(instance, warnings),
        ContentKind::ShaderPack => scan_shader_packs(instance, warnings),
    }
}

fn scan_mods(
    instance: &InstanceLayout,
    warnings: &mut Vec<ContentWarning>,
) -> Result<Vec<LocalContent>, ContentError> {
    let directory = instance.mods_dir();
    let Some(entries) = read_directory(&directory)? else {
        return Ok(Vec::new());
    };
    let mut items = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warnings.push(ContentWarning::Unreadable {
                    path: directory.clone(),
                    message: error.to_string(),
                });
                continue;
            }
        };
        let Some(name) = entry_name(&entry, warnings) else {
            continue;
        };
        if is_hidden(&name) {
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                warnings.push(ContentWarning::Unreadable {
                    path: entry.path(),
                    message: error.to_string(),
                });
                continue;
            }
        };
        if file_type.is_symlink() {
            warnings.push(ContentWarning::Symlink { path: entry.path() });
            continue;
        }
        if file_type.is_file() {
            if ContentKind::Mod.accepts_file_name(&name) {
                push_item(
                    &mut items,
                    warnings,
                    &entry.path(),
                    make_item(ContentKind::Mod, &directory, &entry.path(), None),
                );
            }
            continue;
        }
        if !file_type.is_dir() || name == ".connector" {
            continue;
        }
        let children = match read_directory(&entry.path()) {
            Ok(Some(children)) => children,
            Ok(None) => continue,
            Err(error) => {
                warnings.push(ContentWarning::Unreadable {
                    path: entry.path(),
                    message: error.to_string(),
                });
                continue;
            }
        };
        for child in children {
            let child = match child {
                Ok(child) => child,
                Err(error) => {
                    warnings.push(ContentWarning::Unreadable {
                        path: entry.path(),
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            let Some(child_name) = entry_name(&child, warnings) else {
                continue;
            };
            if is_hidden(&child_name) || !ContentKind::Mod.accepts_file_name(&child_name) {
                continue;
            }
            let child_type = match child.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    warnings.push(ContentWarning::Unreadable {
                        path: child.path(),
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            if child_type.is_symlink() {
                warnings.push(ContentWarning::Symlink { path: child.path() });
                continue;
            }
            if child_type.is_file() {
                push_item(
                    &mut items,
                    warnings,
                    &child.path(),
                    make_item(ContentKind::Mod, &directory, &child.path(), None),
                );
            }
        }
    }
    Ok(items)
}

fn scan_resource_packs(
    instance: &InstanceLayout,
    warnings: &mut Vec<ContentWarning>,
) -> Result<Vec<LocalContent>, ContentError> {
    let directory = instance.resourcepacks_dir();
    let Some(entries) = read_directory(&directory)? else {
        return Ok(Vec::new());
    };
    let options_path = instance.options_path();
    let options = match ResourcePackOptions::load(options_path.clone(), modern_hint(instance)) {
        Ok(options) => options,
        Err(error) => {
            warnings.push(ContentWarning::Options {
                path: options_path,
                message: error.to_string(),
            });
            None
        }
    };
    let enabled = options
        .as_ref()
        .map(ResourcePackOptions::enabled_names)
        .unwrap_or_default();
    let mut items = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warnings.push(ContentWarning::Unreadable {
                    path: directory.clone(),
                    message: error.to_string(),
                });
                continue;
            }
        };
        let Some(name) = entry_name(&entry, warnings) else {
            continue;
        };
        if is_hidden(&name) {
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                warnings.push(ContentWarning::Unreadable {
                    path: entry.path(),
                    message: error.to_string(),
                });
                continue;
            }
        };
        if file_type.is_symlink() {
            warnings.push(ContentWarning::Symlink { path: entry.path() });
            continue;
        }
        let accepted = if file_type.is_file() {
            ContentKind::ResourcePack.accepts_file_name(&name)
        } else if file_type.is_dir() {
            is_resource_pack_directory(&entry.path())
        } else {
            false
        };
        if accepted {
            push_item(
                &mut items,
                warnings,
                &entry.path(),
                make_item(
                    ContentKind::ResourcePack,
                    &directory,
                    &entry.path(),
                    Some(enabled.contains(&name)),
                ),
            );
        }
    }
    Ok(items)
}

fn scan_shader_packs(
    instance: &InstanceLayout,
    warnings: &mut Vec<ContentWarning>,
) -> Result<Vec<LocalContent>, ContentError> {
    let directory = instance.shaderpacks_dir();
    let Some(entries) = read_directory(&directory)? else {
        return Ok(Vec::new());
    };
    let mut items = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warnings.push(ContentWarning::Unreadable {
                    path: directory.clone(),
                    message: error.to_string(),
                });
                continue;
            }
        };
        let Some(name) = entry_name(&entry, warnings) else {
            continue;
        };
        if is_hidden(&name) {
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                warnings.push(ContentWarning::Unreadable {
                    path: entry.path(),
                    message: error.to_string(),
                });
                continue;
            }
        };
        if file_type.is_symlink() {
            warnings.push(ContentWarning::Symlink { path: entry.path() });
            continue;
        }
        let accepted = (file_type.is_file() && ContentKind::ShaderPack.accepts_file_name(&name))
            || file_type.is_dir();
        if accepted {
            push_item(
                &mut items,
                warnings,
                &entry.path(),
                make_item(
                    ContentKind::ShaderPack,
                    &directory,
                    &entry.path(),
                    Some(true),
                ),
            );
        }
    }
    Ok(items)
}

fn push_item(
    items: &mut Vec<LocalContent>,
    warnings: &mut Vec<ContentWarning>,
    path: &Path,
    item: Result<LocalContent, ContentError>,
) {
    match item {
        Ok(item) => items.push(item),
        Err(ContentError::Layout(error)) => warnings.push(ContentWarning::InvalidName {
            path: path.to_path_buf(),
            reason: error.to_string(),
        }),
        Err(error) => warnings.push(ContentWarning::Unreadable {
            path: path.to_path_buf(),
            message: error.to_string(),
        }),
    }
}

fn make_item(
    kind: ContentKind,
    directory: &Path,
    path: &Path,
    enabled: Option<bool>,
) -> Result<LocalContent, ContentError> {
    let relative_path = relative_path(directory, path)?;
    let metadata = fs::metadata(path).map_err(|source| ContentError::Io {
        operation: "inspect content entry",
        path: path.to_path_buf(),
        source,
    })?;
    let is_directory = metadata.is_dir();
    let enabled = enabled.unwrap_or_else(|| {
        let file_name = relative_path
            .as_str()
            .rsplit('/')
            .next()
            .unwrap_or_else(|| relative_path.as_str());
        !strip_disabled_suffix(file_name).1
    });
    Ok(LocalContent {
        kind,
        relative_path,
        path: path.to_path_buf(),
        size: if is_directory {
            None
        } else {
            Some(metadata.len())
        },
        enabled,
        is_directory,
    })
}

fn toggle_mod(directory: &Path, path: &Path, enabled: bool) -> Result<LocalContent, ContentError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| ContentError::Io {
        operation: "inspect mod before toggling",
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(ContentError::UnsupportedOperation {
            kind: ContentKind::Mod,
            operation: "toggle a mod directory",
        });
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ContentError::UnsafePath {
            path: path.to_path_buf(),
            reason: "file name is not valid UTF-8".to_owned(),
        })?;
    let (base_name, _) = strip_disabled_suffix(file_name);
    let desired = if enabled {
        base_name.to_owned()
    } else {
        format!("{base_name}.disabled")
    };
    if desired == file_name {
        return make_item(ContentKind::Mod, directory, path, Some(enabled));
    }
    let parent = path.parent().ok_or_else(|| ContentError::UnsafePath {
        path: path.to_path_buf(),
        reason: "mod path has no parent directory".to_owned(),
    })?;
    let destination = parent.join(&desired);
    match fs::symlink_metadata(&destination) {
        Ok(_) => {
            return Err(ContentError::Conflict { path: destination });
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(ContentError::Io {
                operation: "inspect mod toggle destination",
                path: destination,
                source,
            });
        }
    }
    fs::rename(path, &destination).map_err(|source| ContentError::Io {
        operation: "toggle mod",
        path: path.to_path_buf(),
        source,
    })?;
    make_item(ContentKind::Mod, directory, &destination, Some(enabled))
}

fn relative_path(directory: &Path, path: &Path) -> Result<RepositoryPath, ContentError> {
    let relative = path
        .strip_prefix(directory)
        .map_err(|_| ContentError::UnsafePath {
            path: path.to_path_buf(),
            reason: "content entry is outside its content directory".to_owned(),
        })?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str().ok_or_else(|| ContentError::UnsafePath {
                    path: path.to_path_buf(),
                    reason: "path component is not valid UTF-8".to_owned(),
                })?;
                parts.push(value);
            }
            _ => {
                return Err(ContentError::UnsafePath {
                    path: path.to_path_buf(),
                    reason: "path contains a non-normal component".to_owned(),
                });
            }
        }
    }
    if parts.is_empty() {
        return Err(ContentError::UnsafePath {
            path: path.to_path_buf(),
            reason: "content entry has no relative path".to_owned(),
        });
    }
    RepositoryPath::new(parts.join("/")).map_err(ContentError::from)
}

fn ensure_safe_existing(
    directory: &Path,
    relative_path: &RepositoryPath,
) -> Result<PathBuf, ContentError> {
    let parts = relative_path.as_str().split('/').collect::<Vec<_>>();
    let mut current = directory.to_path_buf();
    for (index, part) in parts.iter().enumerate() {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(ContentError::Symlink { path: current });
                }
                if index + 1 < parts.len() && !metadata.is_dir() {
                    return Err(ContentError::UnsafePath {
                        path: current,
                        reason: "a parent content entry is not a directory".to_owned(),
                    });
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(ContentError::NotFound { path: current });
            }
            Err(source) => {
                return Err(ContentError::Io {
                    operation: "inspect content entry",
                    path: current,
                    source,
                });
            }
        }
    }
    Ok(current)
}

fn ensure_content_directory(path: &Path) -> Result<(), ContentError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(ContentError::Symlink {
                    path: path.to_path_buf(),
                });
            }
            if !metadata.is_dir() {
                return Err(ContentError::NotADirectory {
                    path: path.to_path_buf(),
                });
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir_all(path)
            .map_err(|source| ContentError::Io {
                operation: "create content directory",
                path: path.to_path_buf(),
                source,
            }),
        Err(source) => Err(ContentError::Io {
            operation: "inspect content directory",
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn read_directory(path: &Path) -> Result<Option<fs::ReadDir>, ContentError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(ContentError::Symlink {
                    path: path.to_path_buf(),
                });
            }
            if !metadata.is_dir() {
                return Err(ContentError::NotADirectory {
                    path: path.to_path_buf(),
                });
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ContentError::Io {
                operation: "inspect content directory",
                path: path.to_path_buf(),
                source,
            });
        }
    }
    fs::read_dir(path)
        .map(Some)
        .map_err(|source| ContentError::Io {
            operation: "read content directory",
            path: path.to_path_buf(),
            source,
        })
}

fn entry_name(entry: &fs::DirEntry, warnings: &mut Vec<ContentWarning>) -> Option<String> {
    match entry.file_name().into_string() {
        Ok(name) => Some(name),
        Err(_) => {
            warnings.push(ContentWarning::InvalidName {
                path: entry.path(),
                reason: "file name is not valid UTF-8".to_owned(),
            });
            None
        }
    }
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

fn is_resource_pack_directory(path: &Path) -> bool {
    match fs::symlink_metadata(path.join("pack.mcmeta")) {
        Ok(metadata) => metadata.is_file(),
        Err(_) => false,
    }
}

fn exists_case_insensitive(directory: &Path, file_name: &str) -> Result<bool, ContentError> {
    let target = file_name.to_lowercase();
    let entries = fs::read_dir(directory).map_err(|source| ContentError::Io {
        operation: "read content directory",
        path: directory.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| ContentError::Io {
            operation: "read content directory entry",
            path: directory.to_path_buf(),
            source,
        })?;
        if entry.file_name().to_string_lossy().to_lowercase() == target {
            return Ok(true);
        }
    }
    Ok(false)
}

fn unique_name(directory: &Path, file_name: &str) -> Result<String, ContentError> {
    let (without_disabled, disabled) = strip_disabled_suffix(file_name);
    let (stem, extension) = split_extension(without_disabled);
    for suffix in 0..10_000 {
        let candidate = if suffix == 0 {
            file_name.to_owned()
        } else {
            let base = if extension.is_empty() {
                format!("{stem} ({suffix})")
            } else {
                format!("{stem} ({suffix}).{extension}")
            };
            if disabled {
                format!("{base}.disabled")
            } else {
                base
            }
        };
        if !exists_case_insensitive(directory, &candidate)? {
            return Ok(candidate);
        }
    }
    Err(ContentError::Conflict {
        path: directory.join(file_name),
    })
}

fn split_extension(file_name: &str) -> (&str, &str) {
    match file_name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() => (stem, extension),
        _ => (file_name, ""),
    }
}

fn file_name(relative_path: &RepositoryPath) -> &str {
    relative_path
        .as_str()
        .rsplit('/')
        .next()
        .unwrap_or_else(|| relative_path.as_str())
}

/// Moves an enabled resource pack in the `options.txt` priority list.
///
/// Packs later in `resourcePacks` override earlier ones. Moving a pack that is
/// not enabled returns [`ContentError::ResourcePackNotEnabled`].
pub fn move_resource_pack(
    instance: &InstanceLayout,
    relative_path: &RepositoryPath,
    order: ResourcePackOrder,
) -> Result<(), ContentError> {
    let directory = instance.resourcepacks_dir();
    ensure_safe_existing(&directory, relative_path)?;
    let file_name = file_name(relative_path).to_owned();
    let options_path = instance.options_path();
    let modern = modern_hint(instance);
    let mut options = ResourcePackOptions::load(options_path.clone(), modern)?
        .unwrap_or_else(|| ResourcePackOptions::new(options_path, modern));
    options.move_entry(&file_name, order)?;
    options.save()
}

fn modern_hint(instance: &InstanceLayout) -> bool {
    let version = instance
        .load_manifest()
        .ok()
        .flatten()
        .map(|manifest| {
            manifest
                .resolved_minecraft_version(instance.id())
                .to_owned()
        })
        .unwrap_or_else(|| instance.id().as_str().to_owned());
    modern_resource_pack_list(&version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ContentWarning;
    use rmcl_instance::{IsolationMode, MinecraftLayout, VersionId};
    use tempfile::TempDir;

    fn instance(root: &Path) -> InstanceLayout {
        MinecraftLayout::new(root)
            .instance(VersionId::new("1.21.1").unwrap(), IsolationMode::Isolated)
    }

    fn write(path: &Path, contents: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn item<'a>(library: &'a ContentLibrary, name: &str) -> &'a LocalContent {
        library
            .items
            .iter()
            .find(|item| item.relative_path.as_str() == name)
            .unwrap()
    }

    #[test]
    fn scans_mods_including_nested_and_disabled_entries() {
        let temporary = TempDir::new().unwrap();
        let instance = instance(temporary.path());
        write(&instance.mods_dir().join("alpha.jar"), b"alpha");
        write(&instance.mods_dir().join("beta.jar.disabled"), b"beta");
        write(&instance.mods_dir().join("sub/gamma.zip"), b"gamma");
        write(&instance.mods_dir().join("notes.txt"), b"notes");
        write(&instance.mods_dir().join(".hidden.jar"), b"hidden");
        write(
            &instance.mods_dir().join(".connector/dependency.jar"),
            b"hidden",
        );

        let library = scan_library(&instance);

        assert_eq!(library.count(ContentKind::Mod), 3);
        assert_eq!(item(&library, "alpha.jar").display_name(), "alpha.jar");
        assert!(item(&library, "alpha.jar").enabled);
        assert!(!item(&library, "beta.jar.disabled").enabled);
        assert_eq!(
            item(&library, "beta.jar.disabled").display_name(),
            "beta.jar"
        );
        assert_eq!(item(&library, "sub/gamma.zip").display_name(), "gamma.zip");
        assert!(item(&library, "sub/gamma.zip").enabled);
        assert!(library.warnings.is_empty());
    }

    #[test]
    fn import_renames_conflicts_and_rejects_unsupported_files() {
        let temporary = TempDir::new().unwrap();
        let root = temporary.path().join(".minecraft");
        let instance = instance(&root);
        let source_directory = temporary.path().join("downloads");
        let source = source_directory.join("mod.jar");
        write(&source, b"mod");

        let first =
            import_file(&instance, ContentKind::Mod, &source, ConflictPolicy::Rename).unwrap();
        let second =
            import_file(&instance, ContentKind::Mod, &source, ConflictPolicy::Rename).unwrap();

        assert_eq!(first.relative_path.as_str(), "mod.jar");
        assert_eq!(second.relative_path.as_str(), "mod (1).jar");
        assert_eq!(
            std::fs::read(instance.mods_dir().join("mod.jar")).unwrap(),
            b"mod"
        );
        assert_eq!(
            std::fs::read(instance.mods_dir().join("mod (1).jar")).unwrap(),
            b"mod"
        );

        let disabled_source = source_directory.join("disabled.jar.disabled");
        write(&disabled_source, b"disabled");
        let disabled = import_file(
            &instance,
            ContentKind::Mod,
            &disabled_source,
            ConflictPolicy::Rename,
        )
        .unwrap();
        assert!(!disabled.enabled);
        assert_eq!(disabled.relative_path.as_str(), "disabled.jar.disabled");

        let text = source_directory.join("notes.txt");
        write(&text, b"notes");
        assert!(matches!(
            import_file(&instance, ContentKind::Mod, &text, ConflictPolicy::Rename),
            Err(ContentError::UnsupportedFile { .. })
        ));
        assert!(matches!(
            import_file(&instance, ContentKind::Mod, &source, ConflictPolicy::Reject),
            Err(ContentError::Conflict { .. })
        ));
    }

    #[test]
    fn replacement_preserves_path_and_disabled_state() {
        let temporary = TempDir::new().unwrap();
        let instance = instance(temporary.path());
        let destination = instance.mods_dir().join("example.jar.disabled");
        write(&destination, b"old");
        let relative = RepositoryPath::new("example.jar.disabled").unwrap();
        let source = temporary.path().join("new.jar");
        write(&source, b"new");

        let replaced =
            replace_content_file(&instance, ContentKind::Mod, &source, &relative).unwrap();
        assert_eq!(replaced.relative_path, relative);
        assert!(!replaced.enabled);
        assert_eq!(std::fs::read(&destination).unwrap(), b"new");

        let missing_source = temporary.path().join("missing.jar");
        assert!(
            replace_content_file(&instance, ContentKind::Mod, &missing_source, &relative).is_err()
        );
        assert_eq!(std::fs::read(&destination).unwrap(), b"new");
        assert!(
            replace_content_file(
                &instance,
                ContentKind::Mod,
                &source,
                &RepositoryPath::new("missing.jar").unwrap(),
            )
            .is_err()
        );
    }

    #[test]
    fn toggles_mods_with_the_disabled_suffix() {
        let temporary = TempDir::new().unwrap();
        let instance = instance(temporary.path());
        let path = instance.mods_dir().join("example.jar");
        write(&path, b"mod");
        let relative = RepositoryPath::new("example.jar").unwrap();

        let disabled = set_content_enabled(&instance, ContentKind::Mod, &relative, false).unwrap();

        assert!(!disabled.enabled);
        assert!(!path.exists());
        assert!(instance.mods_dir().join("example.jar.disabled").is_file());

        let relative = RepositoryPath::new("example.jar.disabled").unwrap();
        let enabled = set_content_enabled(&instance, ContentKind::Mod, &relative, true).unwrap();

        assert!(enabled.enabled);
        assert!(path.is_file());
        assert!(!instance.mods_dir().join("example.jar.disabled").exists());
    }

    #[test]
    fn resource_pack_options_control_enabled_state_and_deletion() {
        let temporary = TempDir::new().unwrap();
        let instance = instance(temporary.path());
        write(&instance.resourcepacks_dir().join("pack.zip"), b"pack");
        let folder = instance.resourcepacks_dir().join("Folder Pack");
        write(&folder.join("pack.mcmeta"), b"{}");
        write(
            &instance.options_path(),
            b"language:zh_cn\nresourcePacks:[\"file/pack.zip\"]\n",
        );

        let library = scan_library(&instance);
        assert!(item(&library, "pack.zip").enabled);
        assert!(!item(&library, "Folder Pack").enabled);
        assert!(item(&library, "Folder Pack").is_directory);

        set_content_enabled(
            &instance,
            ContentKind::ResourcePack,
            &RepositoryPath::new("Folder Pack").unwrap(),
            true,
        )
        .unwrap();
        let options = std::fs::read_to_string(instance.options_path()).unwrap();
        assert!(options.contains("language:zh_cn"));
        assert!(options.contains("resourcePacks:[\"file/pack.zip\",\"file/Folder Pack\"]"));

        delete_content(
            &instance,
            ContentKind::ResourcePack,
            &RepositoryPath::new("pack.zip").unwrap(),
        )
        .unwrap();
        assert!(!instance.resourcepacks_dir().join("pack.zip").exists());
        let options = std::fs::read_to_string(instance.options_path()).unwrap();
        assert!(!options.contains("pack.zip"));
        assert!(options.contains("Folder Pack"));

        delete_content(
            &instance,
            ContentKind::ResourcePack,
            &RepositoryPath::new("Folder Pack").unwrap(),
        )
        .unwrap();
        assert!(!folder.exists());
        let options = std::fs::read_to_string(instance.options_path()).unwrap();
        assert!(!options.contains("Folder Pack"));
    }

    #[test]
    fn moves_resource_pack_priority_and_rejects_disabled_packs() {
        let temporary = TempDir::new().unwrap();
        let instance = instance(temporary.path());
        write(&instance.resourcepacks_dir().join("a.zip"), b"a");
        write(&instance.resourcepacks_dir().join("b.zip"), b"b");
        write(&instance.resourcepacks_dir().join("disabled.zip"), b"d");
        write(
            &instance.options_path(),
            b"resourcePacks:[\"file/a.zip\",\"file/b.zip\"]\n",
        );

        move_resource_pack(
            &instance,
            &RepositoryPath::new("a.zip").unwrap(),
            ResourcePackOrder::HigherPriority,
        )
        .unwrap();
        assert!(
            std::fs::read_to_string(instance.options_path())
                .unwrap()
                .contains("resourcePacks:[\"file/b.zip\",\"file/a.zip\"]")
        );

        assert!(matches!(
            move_resource_pack(
                &instance,
                &RepositoryPath::new("disabled.zip").unwrap(),
                ResourcePackOrder::HigherPriority,
            ),
            Err(ContentError::ResourcePackNotEnabled { .. })
        ));
    }

    #[test]
    fn malformed_options_warn_and_block_toggles_without_rewriting() {
        let temporary = TempDir::new().unwrap();
        let instance = instance(temporary.path());
        write(&instance.resourcepacks_dir().join("pack.zip"), b"pack");
        write(&instance.options_path(), b"resourcePacks:not-json\n");

        let library = scan_library(&instance);
        assert!(!item(&library, "pack.zip").enabled);
        assert!(
            library
                .warnings
                .iter()
                .any(|warning| matches!(warning, ContentWarning::Options { .. }))
        );

        let result = set_content_enabled(
            &instance,
            ContentKind::ResourcePack,
            &RepositoryPath::new("pack.zip").unwrap(),
            true,
        );
        assert!(matches!(result, Err(ContentError::Options { .. })));
        assert_eq!(
            std::fs::read_to_string(instance.options_path()).unwrap(),
            "resourcePacks:not-json\n"
        );
    }

    #[test]
    fn shader_packs_scan_files_and_directories_without_toggle() {
        let temporary = TempDir::new().unwrap();
        let instance = instance(temporary.path());
        write(&instance.shaderpacks_dir().join("shader.zip"), b"shader");
        write(
            &instance
                .shaderpacks_dir()
                .join("Folder Shader/shaders/final.fsh"),
            b"shader",
        );

        let library = scan_library(&instance);

        assert_eq!(library.count(ContentKind::ShaderPack), 2);
        assert!(
            library
                .items_for(ContentKind::ShaderPack)
                .all(|item| item.enabled && !item.can_toggle())
        );
        assert!(matches!(
            set_content_enabled(
                &instance,
                ContentKind::ShaderPack,
                &RepositoryPath::new("shader.zip").unwrap(),
                false,
            ),
            Err(ContentError::UnsupportedOperation {
                kind: ContentKind::ShaderPack,
                ..
            })
        ));
    }

    #[test]
    fn deleting_mods_and_directories_is_explicit() {
        let temporary = TempDir::new().unwrap();
        let instance = instance(temporary.path());
        write(&instance.mods_dir().join("example.jar"), b"mod");
        let directory = instance.mods_dir().join("folder");
        std::fs::create_dir_all(&directory).unwrap();

        delete_content(
            &instance,
            ContentKind::Mod,
            &RepositoryPath::new("example.jar").unwrap(),
        )
        .unwrap();
        assert!(!instance.mods_dir().join("example.jar").exists());
        assert!(matches!(
            delete_content(
                &instance,
                ContentKind::Mod,
                &RepositoryPath::new("folder").unwrap(),
            ),
            Err(ContentError::UnsupportedOperation { .. })
        ));
        assert!(directory.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_skipped_and_cannot_be_deleted() {
        use std::os::unix::fs::symlink;

        let temporary = TempDir::new().unwrap();
        let instance = instance(temporary.path());
        std::fs::create_dir_all(instance.mods_dir()).unwrap();
        let outside = temporary.path().join("outside.jar");
        write(&outside, b"outside");
        let link = instance.mods_dir().join("link.jar");
        symlink(&outside, &link).unwrap();

        let library = scan_library(&instance);
        assert_eq!(library.count(ContentKind::Mod), 0);
        assert!(
            library
                .warnings
                .iter()
                .any(|warning| matches!(warning, ContentWarning::Symlink { .. }))
        );
        assert!(matches!(
            delete_content(
                &instance,
                ContentKind::Mod,
                &RepositoryPath::new("link.jar").unwrap(),
            ),
            Err(ContentError::Symlink { .. })
        ));
        assert!(outside.is_file());
    }
}
