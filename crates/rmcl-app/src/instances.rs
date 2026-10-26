use std::path::{Path, PathBuf};

use rmcl_instance::{InstanceLayout, InstanceManifest, IsolationMode, MinecraftLayout, VersionId};
use tokio_util::sync::CancellationToken;

use crate::AppError;

/// Moves an existing legacy instance to an isolated game directory.
///
/// The instance directory and version JSON are never replaced. When
/// `copy_global_content` is true, `mods/`, `resourcepacks/` and `shaderpacks/`
/// from the shared repository are copied into the instance without overwriting
/// existing files or following symlinks. The manifest is written last, so a
/// failed copy leaves the instance in its previous shared mode.
pub async fn migrate_instance_to_isolated(
    root: impl AsRef<Path>,
    instance_id: &str,
    copy_global_content: bool,
) -> Result<(), AppError> {
    let root = std::path::absolute(root.as_ref()).map_err(|source| AppError::Io {
        operation: "resolve Minecraft root",
        path: root.as_ref().to_path_buf(),
        source,
    })?;
    let layout = MinecraftLayout::new(root);
    let id = VersionId::new(instance_id)?;
    let probe = layout.instance(id.clone(), IsolationMode::Shared);
    let manifest = probe.load_manifest()?;
    let instance = layout.instance(id.clone(), IsolationMode::Isolated);

    if !instance.version_json_path().is_file() {
        return Err(AppError::VersionNotFound(instance_id.to_owned()));
    }
    let metadata = std::fs::symlink_metadata(instance.root()).map_err(|source| AppError::Io {
        operation: "inspect instance directory before isolation",
        path: instance.root(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(AppError::InvalidRequest(
            "a symlinked instance directory cannot be migrated to isolation".to_owned(),
        ));
    }

    let already_isolated = manifest
        .as_ref()
        .is_some_and(|manifest| manifest.isolation == IsolationMode::Isolated);
    if !already_isolated && copy_global_content {
        let source_root = layout.root().to_path_buf();
        let target_root = instance.root();
        tokio::task::spawn_blocking(move || copy_shared_content(&source_root, &target_root))
            .await
            .map_err(|error| AppError::BlockingTask(error.to_string()))??;
    }

    let mut manifest =
        manifest.unwrap_or_else(|| InstanceManifest::new(&id, IsolationMode::Isolated));
    manifest.isolation = IsolationMode::Isolated;
    if manifest.name.trim().is_empty() {
        manifest.name = id.as_str().to_owned();
    }
    if manifest.minecraft_version.trim().is_empty() {
        manifest.minecraft_version = id.as_str().to_owned();
    }
    instance.ensure_content_dirs()?;
    instance.write_manifest(&manifest)?;
    Ok(())
}

/// Renames an instance without changing its directory or version ID.
///
/// A blank name is stored as-is: the manifest then falls back to the directory
/// ID when displayed. Legacy versions without a manifest receive one using
/// their current isolation mode.
pub fn rename_instance(instance: &InstanceLayout, name: &str) -> Result<(), AppError> {
    if !instance.version_json_path().is_file() {
        return Err(AppError::VersionNotFound(instance.id().as_str().to_owned()));
    }
    let mut manifest = instance
        .load_manifest()?
        .unwrap_or_else(|| InstanceManifest::new(instance.id(), instance.isolation()));
    manifest.name = name.trim().to_owned();
    instance.write_manifest(&manifest)?;
    Ok(())
}

/// Sets per-instance memory and window overrides.
///
/// `memory_mb = None` leaves the JVM default. Window dimensions must be either
/// both present or both absent. Existing manifest fields are preserved.
pub fn set_instance_launch_settings(
    instance: &InstanceLayout,
    memory_mb: Option<u32>,
    window_width: Option<u32>,
    window_height: Option<u32>,
) -> Result<(), AppError> {
    if !instance.version_json_path().is_file() {
        return Err(AppError::VersionNotFound(instance.id().as_str().to_owned()));
    }
    if let Some(memory) = memory_mb
        && !(256..=262_144).contains(&memory)
    {
        return Err(AppError::InvalidRequest(
            "instance memory must be between 256 and 262144 MiB".to_owned(),
        ));
    }
    let window = match (window_width, window_height) {
        (Some(width), Some(height))
            if (320..=16_384).contains(&width) && (320..=16_384).contains(&height) =>
        {
            Some((width, height))
        }
        (None, None) => None,
        _ => {
            return Err(AppError::InvalidRequest(
                "instance window width and height must both be present and within 320..=16384"
                    .to_owned(),
            ));
        }
    };
    let mut manifest = instance
        .load_manifest()?
        .unwrap_or_else(|| InstanceManifest::new(instance.id(), instance.isolation()));
    manifest.max_memory_mb = memory_mb;
    manifest.window_width = window.map(|(width, _)| width);
    manifest.window_height = window.map(|(_, height)| height);
    instance.write_manifest(&manifest)?;
    Ok(())
}

/// Sets or clears the explicit Java executable stored in an instance manifest.
///
/// Existing manifest fields (name, isolation, loader) are preserved. Legacy
/// versions without a manifest receive one using their current isolation mode,
/// so changing Java never silently moves the game directory.
pub fn set_instance_java(instance: &InstanceLayout, java: Option<PathBuf>) -> Result<(), AppError> {
    if !instance.version_json_path().is_file() {
        return Err(AppError::VersionNotFound(instance.id().as_str().to_owned()));
    }
    let mut manifest = instance
        .load_manifest()?
        .unwrap_or_else(|| InstanceManifest::new(instance.id(), instance.isolation()));
    manifest.java = java;
    instance.write_manifest(&manifest)?;
    Ok(())
}

fn copy_shared_content(source_root: &Path, target_root: &Path) -> Result<(), AppError> {
    for directory in ["mods", "resourcepacks", "shaderpacks"] {
        let source = source_root.join(directory);
        let metadata = match std::fs::symlink_metadata(&source) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(AppError::Io {
                    operation: "inspect shared content directory",
                    path: source,
                    source: error,
                });
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        copy_missing_tree(&source, &target_root.join(directory))?;
    }
    Ok(())
}

fn copy_missing_tree(source: &Path, target: &Path) -> Result<(), AppError> {
    std::fs::create_dir_all(target).map_err(|source| AppError::Io {
        operation: "create isolated content directory",
        path: target.to_path_buf(),
        source,
    })?;
    let entries = std::fs::read_dir(source).map_err(|error| AppError::Io {
        operation: "read shared content directory",
        path: source.to_path_buf(),
        source: error,
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| AppError::Io {
            operation: "read shared content entry",
            path: source.to_path_buf(),
            source: error,
        })?;
        let file_type = entry.file_type().map_err(|source| AppError::Io {
            operation: "inspect shared content entry",
            path: entry.path(),
            source,
        })?;
        if file_type.is_symlink() {
            continue;
        }
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if file_type.is_dir() {
            copy_missing_tree(&source_path, &target_path)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        match std::fs::symlink_metadata(&target_path) {
            Ok(_) => continue,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(AppError::Io {
                    operation: "inspect isolated content entry",
                    path: target_path,
                    source,
                });
            }
        }
        std::fs::copy(&source_path, &target_path).map_err(|source| AppError::Io {
            operation: "copy shared content into isolated instance",
            path: target_path,
            source,
        })?;
    }
    Ok(())
}

const SHARED_CONTENT_DIRS: [&str; 8] = [
    "mods",
    "resourcepacks",
    "shaderpacks",
    "saves",
    "config",
    "defaultconfigs",
    "scripts",
    "kubejs",
];
const SHARED_CONTENT_FILES: [&str; 3] = ["options.txt", "servers.dat", "realms_persistence.json"];

/// Copies game-owned content from an existing instance into a newly created
/// isolated instance. Repository files, version JSON/JAR, natives and logs are
/// never copied. Existing target files are kept.
pub(crate) fn copy_instance_game_content(
    source_game_dir: &Path,
    target_game_dir: &Path,
    source_isolated: bool,
    source_id: &str,
    cancellation: &CancellationToken,
) -> Result<usize, AppError> {
    if source_game_dir == target_game_dir {
        return Err(AppError::InvalidRequest(
            "source and target instance game directories must differ".to_owned(),
        ));
    }
    check_copy_cancelled(cancellation)?;
    let mut copied = 0;
    if source_isolated {
        let entries = std::fs::read_dir(source_game_dir).map_err(|source| AppError::Io {
            operation: "read source instance game directory",
            path: source_game_dir.to_path_buf(),
            source,
        })?;
        for entry in entries {
            check_copy_cancelled(cancellation)?;
            let entry = entry.map_err(|source| AppError::Io {
                operation: "read source instance content entry",
                path: source_game_dir.to_path_buf(),
                source,
            })?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if is_reserved_instance_entry(name, source_id) {
                continue;
            }
            copied +=
                copy_instance_entry(&entry.path(), &target_game_dir.join(name), cancellation)?;
        }
    } else {
        for directory in SHARED_CONTENT_DIRS {
            check_copy_cancelled(cancellation)?;
            let source = source_game_dir.join(directory);
            let target = target_game_dir.join(directory);
            if source.is_dir() {
                copied += copy_instance_entry(&source, &target, cancellation)?;
            }
        }
        for file in SHARED_CONTENT_FILES {
            check_copy_cancelled(cancellation)?;
            let source = source_game_dir.join(file);
            let target = target_game_dir.join(file);
            if source.is_file() {
                copied += copy_instance_entry(&source, &target, cancellation)?;
            }
        }
    }
    Ok(copied)
}

fn is_reserved_instance_entry(name: &str, source_id: &str) -> bool {
    name == "rmcl.instance.toml"
        || name == "logs"
        || name == "crash-reports"
        || name == "downloads"
        || name == ".DS_Store"
        || name.starts_with("natives-")
        || name == format!("{source_id}.json")
        || name == format!("{source_id}.jar")
}

fn copy_instance_entry(
    source: &Path,
    target: &Path,
    cancellation: &CancellationToken,
) -> Result<usize, AppError> {
    check_copy_cancelled(cancellation)?;
    let metadata = std::fs::symlink_metadata(source).map_err(|source_error| AppError::Io {
        operation: "inspect instance content entry",
        path: source.to_path_buf(),
        source: source_error,
    })?;
    if metadata.file_type().is_symlink() {
        return Ok(0);
    }
    if metadata.is_dir() {
        std::fs::create_dir_all(target).map_err(|source_error| AppError::Io {
            operation: "create copied instance content directory",
            path: target.to_path_buf(),
            source: source_error,
        })?;
        let entries = std::fs::read_dir(source).map_err(|source_error| AppError::Io {
            operation: "read instance content directory",
            path: source.to_path_buf(),
            source: source_error,
        })?;
        let mut copied = 0;
        for entry in entries {
            check_copy_cancelled(cancellation)?;
            let entry = entry.map_err(|source_error| AppError::Io {
                operation: "read instance content entry",
                path: source.to_path_buf(),
                source: source_error,
            })?;
            let name = entry.file_name();
            copied += copy_instance_entry(&entry.path(), &target.join(&name), cancellation)?;
        }
        return Ok(copied);
    }
    if !metadata.is_file() {
        return Ok(0);
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|source_error| AppError::Io {
            operation: "create copied instance content directory",
            path: parent.to_path_buf(),
            source: source_error,
        })?;
    }
    match std::fs::symlink_metadata(target) {
        Ok(_) => return Ok(0),
        Err(source_error) if source_error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source_error) => {
            return Err(AppError::Io {
                operation: "inspect copied instance content entry",
                path: target.to_path_buf(),
                source: source_error,
            });
        }
    }
    std::fs::copy(source, target).map_err(|source_error| AppError::Io {
        operation: "copy instance content",
        path: target.to_path_buf(),
        source: source_error,
    })?;
    Ok(1)
}

fn check_copy_cancelled(cancellation: &CancellationToken) -> Result<(), AppError> {
    if cancellation.is_cancelled() {
        Err(AppError::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_version(root: &Path, id: &str) {
        let directory = root.join("versions").join(id);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join(format!("{id}.json")),
            format!(r#"{{"id":"{id}","type":"release"}}"#),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn migration_copies_shared_content_without_overwriting_instance_files() {
        let temporary = TempDir::new().unwrap();
        let root = temporary.path();
        write_version(root, "1.21.1");
        std::fs::create_dir_all(root.join("mods")).unwrap();
        std::fs::write(root.join("mods/shared.jar"), b"shared").unwrap();
        std::fs::write(root.join("mods/keep.jar"), b"global").unwrap();
        std::fs::create_dir_all(root.join("resourcepacks")).unwrap();
        std::fs::write(root.join("resourcepacks/pack.zip"), b"pack").unwrap();
        std::fs::create_dir_all(root.join("versions/1.21.1/mods")).unwrap();
        std::fs::write(root.join("versions/1.21.1/mods/keep.jar"), b"instance").unwrap();

        migrate_instance_to_isolated(root, "1.21.1", true)
            .await
            .unwrap();

        let instance = root.join("versions/1.21.1");
        assert_eq!(
            std::fs::read(instance.join("mods/shared.jar")).unwrap(),
            b"shared"
        );
        assert_eq!(
            std::fs::read(instance.join("mods/keep.jar")).unwrap(),
            b"instance"
        );
        assert_eq!(
            std::fs::read(instance.join("resourcepacks/pack.zip")).unwrap(),
            b"pack"
        );
        assert!(root.join("mods/shared.jar").is_file());
        let manifest = MinecraftLayout::new(root)
            .instance(VersionId::new("1.21.1").unwrap(), IsolationMode::Isolated)
            .load_manifest()
            .unwrap()
            .unwrap();
        assert_eq!(manifest.isolation, IsolationMode::Isolated);
        assert_eq!(manifest.name, "1.21.1");
    }

    #[tokio::test]
    async fn migration_preserves_an_existing_manifest_name() {
        let temporary = TempDir::new().unwrap();
        let root = temporary.path();
        write_version(root, "1.21.1");
        let shared = MinecraftLayout::new(root)
            .instance(VersionId::new("1.21.1").unwrap(), IsolationMode::Shared);
        let mut manifest =
            InstanceManifest::new(&VersionId::new("1.21.1").unwrap(), IsolationMode::Shared);
        manifest.name = "Survival".to_owned();
        shared.write_manifest(&manifest).unwrap();

        migrate_instance_to_isolated(root, "1.21.1", false)
            .await
            .unwrap();

        let manifest = shared.load_manifest().unwrap().unwrap();
        assert_eq!(manifest.name, "Survival");
        assert_eq!(manifest.isolation, IsolationMode::Isolated);
    }

    #[test]
    fn java_override_preserves_instance_fields_and_shared_isolation() {
        let temporary = TempDir::new().unwrap();
        let root = temporary.path();
        write_version(root, "1.21.1");
        let layout = MinecraftLayout::new(root);
        let shared = layout.instance(VersionId::new("1.21.1").unwrap(), IsolationMode::Shared);
        let mut manifest =
            InstanceManifest::new(&VersionId::new("1.21.1").unwrap(), IsolationMode::Shared);
        manifest.name = "Survival".to_owned();
        shared.write_manifest(&manifest).unwrap();

        set_instance_java(&shared, Some(PathBuf::from("/opt/java/bin/java"))).unwrap();

        let loaded = shared.load_manifest().unwrap().unwrap();
        assert_eq!(loaded.name, "Survival");
        assert_eq!(loaded.isolation, IsolationMode::Shared);
        assert_eq!(
            loaded.java.as_deref(),
            Some(Path::new("/opt/java/bin/java"))
        );

        set_instance_java(&shared, None).unwrap();
        assert_eq!(shared.load_manifest().unwrap().unwrap().java, None);
    }

    #[test]
    fn rename_preserves_instance_settings_and_falls_back_when_blank() {
        let temporary = TempDir::new().unwrap();
        let root = temporary.path();
        write_version(root, "1.21.1");
        let layout = MinecraftLayout::new(root);
        let instance = layout.instance(VersionId::new("1.21.1").unwrap(), IsolationMode::Isolated);
        let mut manifest =
            InstanceManifest::new(&VersionId::new("1.21.1").unwrap(), IsolationMode::Isolated);
        manifest.loader = Some(rmcl_instance::InstanceLoader {
            kind: rmcl_instance::LoaderKind::Fabric,
            version: "0.16.10".to_owned(),
        });
        manifest.java = Some(PathBuf::from("/opt/java/bin/java"));
        instance.write_manifest(&manifest).unwrap();

        rename_instance(&instance, "  生存世界  ").unwrap();
        let renamed = instance.load_manifest().unwrap().unwrap();
        assert_eq!(renamed.name, "生存世界");
        assert_eq!(renamed.loader, manifest.loader);
        assert_eq!(renamed.java, manifest.java);

        rename_instance(&instance, "   ").unwrap();
        let id = VersionId::new("1.21.1").unwrap();
        assert_eq!(
            instance.load_manifest().unwrap().unwrap().display_name(&id),
            "1.21.1"
        );
    }

    #[test]
    fn launch_settings_validate_and_preserve_other_fields() {
        let temporary = TempDir::new().unwrap();
        let root = temporary.path();
        write_version(root, "1.21.1");
        let instance = MinecraftLayout::new(root)
            .instance(VersionId::new("1.21.1").unwrap(), IsolationMode::Isolated);
        let mut manifest =
            InstanceManifest::new(&VersionId::new("1.21.1").unwrap(), IsolationMode::Isolated);
        manifest.name = "Survival".to_owned();
        manifest.java = Some(PathBuf::from("/opt/java/bin/java"));
        instance.write_manifest(&manifest).unwrap();

        set_instance_launch_settings(&instance, Some(4096), Some(1280), Some(720)).unwrap();
        let loaded = instance.load_manifest().unwrap().unwrap();
        assert_eq!(loaded.max_memory_mb, Some(4096));
        assert_eq!(loaded.window_width, Some(1280));
        assert_eq!(loaded.window_height, Some(720));
        assert_eq!(loaded.name, "Survival");
        assert_eq!(loaded.java, manifest.java);

        set_instance_launch_settings(&instance, None, None, None).unwrap();
        let loaded = instance.load_manifest().unwrap().unwrap();
        assert_eq!(loaded.max_memory_mb, None);
        assert_eq!(loaded.window_width, None);
        assert_eq!(loaded.window_height, None);

        assert!(matches!(
            set_instance_launch_settings(&instance, Some(1), None, None),
            Err(AppError::InvalidRequest(_))
        ));
        assert!(matches!(
            set_instance_launch_settings(&instance, None, Some(1280), None),
            Err(AppError::InvalidRequest(_))
        ));
    }

    #[test]
    fn java_override_rejects_missing_versions() {
        let temporary = TempDir::new().unwrap();
        let layout = MinecraftLayout::new(temporary.path());
        let instance = layout.instance(VersionId::new("missing").unwrap(), IsolationMode::Isolated);

        let error = set_instance_java(&instance, None).unwrap_err();

        assert!(matches!(error, AppError::VersionNotFound(_)));
    }

    #[tokio::test]
    async fn migration_rejects_missing_versions() {
        let temporary = TempDir::new().unwrap();

        let error = migrate_instance_to_isolated(temporary.path(), "1.21.1", false)
            .await
            .unwrap_err();

        assert!(matches!(error, AppError::VersionNotFound(_)));
    }
}
