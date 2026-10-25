use std::path::{Path, PathBuf};

use rmcl_minecraft::{Platform, Sha1Hash};

use crate::{LayoutError, RepositoryPath, VersionId, path::validate_segment};

/// Paths for the standard shared `.minecraft` repository layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinecraftLayout {
    root: PathBuf,
}

impl MinecraftLayout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn versions_dir(&self) -> PathBuf {
        self.root.join("versions")
    }

    pub fn version_dir(&self, id: &VersionId) -> PathBuf {
        self.versions_dir().join(id.as_str())
    }

    pub fn version_json_path(&self, id: &VersionId) -> PathBuf {
        self.version_dir(id).join(format!("{id}.json"))
    }

    pub fn client_jar_path(&self, id: &VersionId) -> PathBuf {
        self.version_dir(id).join(format!("{id}.jar"))
    }

    pub fn natives_dir(&self, id: &VersionId, platform: &Platform) -> PathBuf {
        self.version_dir(id).join(format!(
            "natives-{}-{}",
            platform.os.native_directory_name(),
            platform.arch.as_str()
        ))
    }

    pub fn libraries_dir(&self) -> PathBuf {
        self.root.join("libraries")
    }

    pub fn library_path(&self, path: &RepositoryPath) -> PathBuf {
        path.to_path(self.libraries_dir())
    }

    pub fn assets_dir(&self) -> PathBuf {
        self.root.join("assets")
    }

    pub fn asset_indexes_dir(&self) -> PathBuf {
        self.assets_dir().join("indexes")
    }

    pub fn asset_index_path(&self, id: &str) -> Result<PathBuf, LayoutError> {
        validate_segment(id).map_err(|reason| LayoutError::UnsafePath {
            path: id.to_owned(),
            reason,
        })?;
        Ok(self.asset_indexes_dir().join(format!("{id}.json")))
    }

    pub fn asset_objects_dir(&self) -> PathBuf {
        self.assets_dir().join("objects")
    }

    pub fn asset_object_path(&self, hash: Sha1Hash) -> PathBuf {
        let hash = hash.to_string();
        self.asset_objects_dir().join(&hash[..2]).join(&hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_standard_repository_paths() {
        let layout = MinecraftLayout::new(".minecraft");
        let id = VersionId::new("1.21.1").unwrap();
        let platform = Platform::new(rmcl_minecraft::OperatingSystem::Linux, "6.1", "x86_64");

        assert_eq!(
            layout.version_json_path(&id),
            Path::new(".minecraft/versions/1.21.1/1.21.1.json")
        );
        assert_eq!(
            layout.client_jar_path(&id),
            Path::new(".minecraft/versions/1.21.1/1.21.1.jar")
        );
        assert_eq!(
            layout.natives_dir(&id, &platform),
            Path::new(".minecraft/versions/1.21.1/natives-linux-x86_64")
        );
        assert_eq!(
            layout.asset_index_path("17").unwrap(),
            Path::new(".minecraft/assets/indexes/17.json")
        );
    }

    #[test]
    fn separates_native_directories_by_platform() {
        let layout = MinecraftLayout::new(".minecraft");
        let id = VersionId::new("example").unwrap();
        let windows = Platform::new(rmcl_minecraft::OperatingSystem::Windows, "10", "x86");
        let mac = Platform::new(rmcl_minecraft::OperatingSystem::MacOs, "15", "arm64");

        assert_eq!(
            layout.natives_dir(&id, &windows),
            Path::new(".minecraft/versions/example/natives-windows-x86")
        );
        assert_eq!(
            layout.natives_dir(&id, &mac),
            Path::new(".minecraft/versions/example/natives-macos-aarch64")
        );
    }

    #[test]
    fn maps_asset_objects_by_hash_prefix() {
        let layout = MinecraftLayout::new(".minecraft");
        let hash = "0123456789abcdef0123456789abcdef01234567".parse().unwrap();

        assert_eq!(
            layout.asset_object_path(hash),
            Path::new(".minecraft/assets/objects/01/0123456789abcdef0123456789abcdef01234567")
        );
    }

    #[test]
    fn joins_validated_library_paths_to_libraries() {
        let layout = MinecraftLayout::new(".minecraft");
        let path = RepositoryPath::new("example/lib/1.0/lib-1.0.jar").unwrap();

        assert_eq!(
            layout.library_path(&path),
            Path::new(".minecraft/libraries/example/lib/1.0/lib-1.0.jar")
        );
    }
}
