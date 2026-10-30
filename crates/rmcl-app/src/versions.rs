use std::path::{Path, PathBuf};

use rmcl_install::resolve_version;
use rmcl_instance::{InstanceLayout, InstanceLoader, IsolationMode, MinecraftLayout, VersionId};
use rmcl_minecraft::GameVersion;

use crate::{AppError, RepositoryWriteGuard, try_lock_repository};

/// One valid version directory and its RMCL instance metadata.
///
/// Legacy versions have no manifest and keep using the shared game directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledVersion {
    pub id: VersionId,
    pub name: String,
    pub minecraft_version: String,
    pub version_type: Option<String>,
    pub release_time: Option<String>,
    pub version_dir: PathBuf,
    pub game_dir: PathBuf,
    pub isolation: IsolationMode,
    pub loader: Option<InstanceLoader>,
    /// Explicit Java executable from the manifest; `None` means automatic.
    pub java: Option<PathBuf>,
    pub max_memory_mb: Option<u32>,
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    pub manifest_present: bool,
    pub manifest_error: Option<String>,
    pub client_jar_path: PathBuf,
    pub client_jar_present: bool,
}

impl InstalledVersion {
    pub fn is_release(&self) -> bool {
        self.version_type.as_deref() == Some("release")
    }

    pub fn is_isolated(&self) -> bool {
        self.isolation == IsolationMode::Isolated
    }

    pub fn is_legacy(&self) -> bool {
        !self.manifest_present
    }

    pub fn display_name(&self) -> &str {
        let name = self.name.trim();
        if name.is_empty() {
            self.id.as_str()
        } else {
            name
        }
    }

    /// Rebuilds the instance layout from the scanned metadata.
    pub fn instance_layout(&self, minecraft_root: impl Into<PathBuf>) -> InstanceLayout {
        MinecraftLayout::new(minecraft_root).instance(self.id.clone(), self.isolation)
    }
}

/// Enumerates valid `versions/<id>/<id>.json` entries, newest release first.
///
/// A missing repository or version directory is an empty installation, not an
/// error. Entries with unsafe names, unreadable JSON, or a mismatched ID are
/// skipped because they cannot be launched. A malformed RMCL manifest is
/// surfaced on the entry as `manifest_error` while the version remains usable
/// in shared mode, so one bad instance cannot hide the whole list.
pub fn discover_installed_versions(
    root: impl AsRef<Path>,
) -> Result<Vec<InstalledVersion>, AppError> {
    let layout = MinecraftLayout::new(root.as_ref());
    let versions_dir = layout.versions_dir();
    let entries = match std::fs::read_dir(&versions_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(AppError::Io {
                operation: "read Minecraft versions directory",
                path: versions_dir,
                source,
            });
        }
    };

    let mut versions = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(id) = VersionId::new(name) else {
            continue;
        };
        let json_path = layout.version_json_path(&id);
        let Ok(bytes) = std::fs::read(&json_path) else {
            continue;
        };
        let Ok(json) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let Ok(version) = GameVersion::from_json(json) else {
            continue;
        };
        if version.id != id.as_str() {
            continue;
        }
        let probe = layout.instance(id.clone(), IsolationMode::Shared);
        let (manifest, manifest_present, manifest_error) = match probe.load_manifest() {
            Ok(Some(manifest)) => (Some(manifest), true, None),
            Ok(None) => (None, false, None),
            Err(error) => (None, true, Some(error.to_string())),
        };
        let isolation = manifest
            .as_ref()
            .map_or(IsolationMode::Shared, |manifest| manifest.isolation);
        let instance = layout.instance(id.clone(), isolation);
        let name = manifest.as_ref().map_or_else(
            || id.as_str().to_owned(),
            |manifest| manifest.display_name(&id).to_owned(),
        );
        let minecraft_version = manifest.as_ref().map_or_else(
            || id.as_str().to_owned(),
            |manifest| manifest.resolved_minecraft_version(&id).to_owned(),
        );
        let loader = manifest
            .as_ref()
            .and_then(|manifest| manifest.loader.clone());
        let java = manifest.as_ref().and_then(|manifest| manifest.java.clone());
        let max_memory_mb = manifest
            .as_ref()
            .and_then(|manifest| manifest.max_memory_mb);
        let window_width = manifest.as_ref().and_then(|manifest| manifest.window_width);
        let window_height = manifest
            .as_ref()
            .and_then(|manifest| manifest.window_height);
        let resolved = version
            .inherits_from
            .as_ref()
            .and_then(|_| resolve_version(&layout, &id).ok());
        let jar = resolved
            .as_ref()
            .and_then(|resolved| resolved.jar.clone())
            .or(version.jar);
        let version_type = resolved
            .as_ref()
            .and_then(|resolved| resolved.version_type.clone())
            .or(version.version_type);
        let release_time = resolved
            .as_ref()
            .and_then(|resolved| resolved.release_time.clone())
            .or(version.release_time);
        let jar_id = jar
            .as_deref()
            .and_then(|jar| VersionId::new(jar).ok())
            .unwrap_or_else(|| id.clone());
        let client_jar_path = layout.client_jar_path(&jar_id);
        versions.push(InstalledVersion {
            id,
            name,
            minecraft_version,
            version_type,
            release_time,
            version_dir: instance.root(),
            game_dir: instance.game_dir(),
            isolation,
            loader,
            java,
            max_memory_mb,
            window_width,
            window_height,
            manifest_present,
            manifest_error,
            client_jar_present: client_jar_path.is_file(),
            client_jar_path,
        });
    }

    versions.sort_by(|left, right| {
        right
            .release_time
            .cmp(&left.release_time)
            .then_with(|| left.id.as_str().cmp(right.id.as_str()))
    });
    Ok(versions)
}

/// Deletes one installed version directory after validating its JSON and identity.
///
/// Shared `libraries/` and `assets/` are never removed. Symlinked version
/// directories are rejected instead of followed.
pub async fn delete_installed_version(
    root: impl AsRef<Path>,
    version_id: &str,
) -> Result<(), AppError> {
    let repository_guard = try_lock_repository(root.as_ref())?;
    delete_installed_version_with_guard(root, version_id, &repository_guard).await
}

/// Deletes an installed version while the caller holds the repository write lock.
pub async fn delete_installed_version_with_guard(
    root: impl AsRef<Path>,
    version_id: &str,
    _repository_guard: &RepositoryWriteGuard,
) -> Result<(), AppError> {
    let layout = MinecraftLayout::new(root.as_ref());
    let id = VersionId::new(version_id)?;
    let json_path = layout.version_json_path(&id);
    let bytes = match tokio::fs::read(&json_path).await {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::VersionNotFound(version_id.to_owned()));
        }
        Err(source) => {
            return Err(AppError::Io {
                operation: "read version JSON before deletion",
                path: json_path,
                source,
            });
        }
    };
    let json = String::from_utf8(bytes).map_err(|source| AppError::JsonUtf8 {
        path: json_path.clone(),
        source,
    })?;
    let version = GameVersion::from_json(&json)?;
    if version.id != id.as_str() {
        return Err(AppError::VersionIdMismatch {
            expected: id.as_str().to_owned(),
            actual: version.id,
        });
    }

    let version_dir = layout.version_dir(&id);
    let metadata = match tokio::fs::symlink_metadata(&version_dir).await {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::VersionNotFound(version_id.to_owned()));
        }
        Err(source) => {
            return Err(AppError::Io {
                operation: "inspect version directory before deletion",
                path: version_dir,
                source,
            });
        }
    };
    if !metadata.is_dir() {
        return Err(AppError::VersionNotFound(version_id.to_owned()));
    }

    let directory = version_dir.clone();
    tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&directory))
        .await
        .map_err(|error| AppError::BlockingTask(error.to_string()))?
        .map_err(|source| AppError::Io {
            operation: "delete version directory",
            path: version_dir,
            source,
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_version(root: &Path, id: &str, version_type: &str, release_time: &str, jar: bool) {
        let directory = root.join("versions").join(id);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join(format!("{id}.json")),
            format!(r#"{{"id":"{id}","type":"{version_type}","releaseTime":"{release_time}"}}"#),
        )
        .unwrap();
        if jar {
            std::fs::write(directory.join(format!("{id}.jar")), b"jar").unwrap();
        }
    }

    #[test]
    fn missing_repository_is_an_empty_installation() {
        let temporary = TempDir::new().unwrap();
        let versions = discover_installed_versions(temporary.path().join("missing")).unwrap();

        assert!(versions.is_empty());
    }

    #[test]
    fn discovers_valid_versions_newest_first() {
        let temporary = TempDir::new().unwrap();
        write_version(
            temporary.path(),
            "1.21.1",
            "release",
            "2024-08-08T08:00:00+00:00",
            true,
        );
        write_version(
            temporary.path(),
            "24w14a",
            "snapshot",
            "2024-04-03T12:00:00+00:00",
            false,
        );

        let versions = discover_installed_versions(temporary.path()).unwrap();

        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].id.as_str(), "1.21.1");
        assert!(versions[0].is_release());
        assert!(versions[0].client_jar_present);
        assert!(versions[0].is_legacy());
        assert!(!versions[0].is_isolated());
        assert_eq!(versions[0].name, "1.21.1");
        assert_eq!(versions[0].game_dir, temporary.path());
        assert_eq!(versions[1].id.as_str(), "24w14a");
        assert!(!versions[1].client_jar_present);
    }

    #[test]
    fn loads_manifest_name_loader_and_isolated_game_dir() {
        let temporary = TempDir::new().unwrap();
        write_version(
            temporary.path(),
            "fabric-1.21.1",
            "release",
            "2024-08-08T08:00:00+00:00",
            true,
        );
        let layout = MinecraftLayout::new(temporary.path());
        let instance = layout.instance(
            VersionId::new("fabric-1.21.1").unwrap(),
            IsolationMode::Isolated,
        );
        let mut manifest = rmcl_instance::InstanceManifest::new(
            &VersionId::new("fabric-1.21.1").unwrap(),
            IsolationMode::Isolated,
        );
        manifest.name = "Fabric 生存".to_owned();
        manifest.minecraft_version = "1.21.1".to_owned();
        manifest.loader = Some(rmcl_instance::InstanceLoader {
            kind: rmcl_instance::LoaderKind::Fabric,
            version: "0.16.10".to_owned(),
        });
        manifest.java = Some(PathBuf::from("/opt/java/bin/java"));
        manifest.max_memory_mb = Some(4096);
        manifest.window_width = Some(1280);
        manifest.window_height = Some(720);
        instance.write_manifest(&manifest).unwrap();

        let versions = discover_installed_versions(temporary.path()).unwrap();

        assert_eq!(versions.len(), 1);
        let version = &versions[0];
        assert_eq!(version.display_name(), "Fabric 生存");
        assert_eq!(version.minecraft_version, "1.21.1");
        assert!(version.is_isolated());
        assert!(!version.is_legacy());
        assert_eq!(
            version.game_dir,
            temporary.path().join("versions/fabric-1.21.1")
        );
        assert_eq!(
            version.loader.as_ref().map(|loader| loader.kind.as_str()),
            Some("fabric")
        );
        assert_eq!(
            version.java.as_deref(),
            Some(Path::new("/opt/java/bin/java"))
        );
        assert_eq!(version.max_memory_mb, Some(4096));
        assert_eq!(version.window_width, Some(1280));
        assert_eq!(version.window_height, Some(720));
    }

    #[test]
    fn loader_instance_uses_the_parent_client_jar() {
        let temporary = TempDir::new().unwrap();
        let root = temporary.path();
        write_version(root, "1.21.1", "release", "2024-08-08T08:00:00+00:00", true);
        let loader_dir = root.join("versions/fabric-test");
        std::fs::create_dir_all(&loader_dir).unwrap();
        std::fs::write(
            loader_dir.join("fabric-test.json"),
            br#"{"id":"fabric-test","inheritsFrom":"1.21.1","mainClass":"example.Loader"}"#,
        )
        .unwrap();
        let mut manifest = rmcl_instance::InstanceManifest::new(
            &VersionId::new("fabric-test").unwrap(),
            IsolationMode::Isolated,
        );
        manifest.minecraft_version = "1.21.1".to_owned();
        manifest.loader = Some(rmcl_instance::InstanceLoader {
            kind: rmcl_instance::LoaderKind::Fabric,
            version: "0.16.10".to_owned(),
        });
        MinecraftLayout::new(root)
            .instance(
                VersionId::new("fabric-test").unwrap(),
                IsolationMode::Isolated,
            )
            .write_manifest(&manifest)
            .unwrap();

        let versions = discover_installed_versions(root).unwrap();

        assert_eq!(versions.len(), 2);
        let loader = versions
            .iter()
            .find(|version| version.id.as_str() == "fabric-test")
            .unwrap();
        assert!(loader.client_jar_present);
        assert_eq!(loader.minecraft_version, "1.21.1");
        assert_eq!(loader.version_type.as_deref(), Some("release"));
        assert_eq!(
            loader.release_time.as_deref(),
            Some("2024-08-08T08:00:00+00:00")
        );
    }

    #[test]
    fn malformed_manifest_does_not_hide_the_version() {
        let temporary = TempDir::new().unwrap();
        write_version(
            temporary.path(),
            "1.21.1",
            "release",
            "2024-08-08T08:00:00+00:00",
            true,
        );
        std::fs::write(
            temporary.path().join("versions/1.21.1/rmcl.instance.toml"),
            "schema_version = 1\nisolation = \"broken\"\n",
        )
        .unwrap();

        let versions = discover_installed_versions(temporary.path()).unwrap();

        assert_eq!(versions.len(), 1);
        assert!(versions[0].manifest_present);
        assert!(versions[0].manifest_error.is_some());
        assert!(!versions[0].is_isolated());
    }

    #[test]
    fn skips_unsafe_names_invalid_json_and_mismatched_ids() {
        let temporary = TempDir::new().unwrap();
        write_version(
            temporary.path(),
            "valid",
            "release",
            "2024-01-01T00:00:00+00:00",
            true,
        );
        let invalid = temporary.path().join("versions").join("broken");
        std::fs::create_dir_all(&invalid).unwrap();
        std::fs::write(invalid.join("broken.json"), b"{").unwrap();
        let mismatched = temporary.path().join("versions").join("mismatched");
        std::fs::create_dir_all(&mismatched).unwrap();
        std::fs::write(
            mismatched.join("mismatched.json"),
            br#"{"id":"different","type":"release"}"#,
        )
        .unwrap();
        std::fs::create_dir_all(temporary.path().join("versions").join("not a json file")).unwrap();

        let versions = discover_installed_versions(temporary.path()).unwrap();

        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].id.as_str(), "valid");
    }

    #[test]
    fn a_valid_directory_without_json_is_not_an_installed_version() {
        let temporary = TempDir::new().unwrap();
        std::fs::create_dir_all(temporary.path().join("versions/1.20.1")).unwrap();

        assert!(
            discover_installed_versions(temporary.path())
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn deletes_one_version_and_keeps_shared_files() {
        let temporary = TempDir::new().unwrap();
        let root = temporary.path();
        write_version(root, "1.21.1", "release", "2024-08-08T08:00:00+00:00", true);
        std::fs::create_dir_all(root.join("libraries/example")).unwrap();
        std::fs::write(root.join("libraries/example/lib.jar"), b"library").unwrap();
        std::fs::create_dir_all(root.join("assets/objects/aa")).unwrap();
        std::fs::write(root.join("assets/objects/aa/hash"), b"asset").unwrap();

        delete_installed_version(root, "1.21.1").await.unwrap();

        assert!(!root.join("versions/1.21.1").exists());
        assert!(root.join("libraries/example/lib.jar").exists());
        assert!(root.join("assets/objects/aa/hash").exists());
    }

    #[tokio::test]
    async fn deletion_requires_a_matching_version_json() {
        let temporary = TempDir::new().unwrap();
        let root = temporary.path();
        std::fs::create_dir_all(root.join("versions/1.21.1")).unwrap();

        let error = delete_installed_version(root, "1.21.1").await.unwrap_err();

        assert!(matches!(error, AppError::VersionNotFound(_)));
        assert!(root.join("versions/1.21.1").exists());
    }

    #[tokio::test]
    async fn deletion_rejects_unsafe_ids() {
        let temporary = TempDir::new().unwrap();

        let error = delete_installed_version(temporary.path(), "../escape")
            .await
            .unwrap_err();

        assert!(matches!(error, AppError::Layout(_)));
    }
}
