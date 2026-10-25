use std::path::{Path, PathBuf};

use rmcl_instance::{MinecraftLayout, VersionId};
use rmcl_minecraft::GameVersion;

use crate::AppError;

/// One valid version directory discovered in a standard Minecraft repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledVersion {
    pub id: VersionId,
    pub version_type: Option<String>,
    pub release_time: Option<String>,
    pub version_dir: PathBuf,
    pub client_jar_present: bool,
}

impl InstalledVersion {
    pub fn is_release(&self) -> bool {
        self.version_type.as_deref() == Some("release")
    }
}

/// Enumerates valid `versions/<id>/<id>.json` entries, newest release first.
///
/// A missing repository or version directory is an empty installation, not an
/// error. Entries with unsafe names, unreadable JSON, or a mismatched ID are
/// skipped because they cannot be launched as standalone Vanilla versions.
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
        versions.push(InstalledVersion {
            version_dir: layout.version_dir(&id),
            client_jar_present: layout.client_jar_path(&id).is_file(),
            id,
            version_type: version.version_type,
            release_time: version.release_time,
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
        assert_eq!(versions[1].id.as_str(), "24w14a");
        assert!(!versions[1].client_jar_present);
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
