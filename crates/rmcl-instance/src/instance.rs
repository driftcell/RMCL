use std::{io::Write, path::PathBuf};

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use thiserror::Error;

use crate::{LayoutError, MinecraftLayout, VersionId};

/// Schema version written into every RMCL instance manifest.
pub const INSTANCE_MANIFEST_SCHEMA_VERSION: u32 = 1;

/// File name of the RMCL-owned instance manifest inside an instance root.
pub const INSTANCE_MANIFEST_FILE: &str = "rmcl.instance.toml";

/// Whether an instance uses its own game directory or the shared repository root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IsolationMode {
    /// Mods, resource packs, shader packs, saves and game settings live under the instance root.
    #[default]
    Isolated,
    /// The game directory is the shared Minecraft repository root.
    Shared,
}

/// A mod loader recorded in an instance manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceLoader {
    pub kind: LoaderKind,
    pub version: String,
}

/// Supported mod loader families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoaderKind {
    Fabric,
    Quilt,
    Forge,
    NeoForge,
}

impl LoaderKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fabric => "fabric",
            Self::Quilt => "quilt",
            Self::Forge => "forge",
            Self::NeoForge => "neoforge",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Fabric => "Fabric",
            Self::Quilt => "Quilt",
            Self::Forge => "Forge",
            Self::NeoForge => "NeoForge",
        }
    }
}

/// RMCL-owned metadata for one instance.
///
/// The instance directory name is the identity; it is intentionally not stored
/// here so renaming or moving a directory cannot leave a stale duplicate ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InstanceManifest {
    pub schema_version: u32,
    pub name: String,
    pub minecraft_version: String,
    pub isolation: IsolationMode,
    pub loader: Option<InstanceLoader>,
}

impl Default for InstanceManifest {
    fn default() -> Self {
        Self {
            schema_version: INSTANCE_MANIFEST_SCHEMA_VERSION,
            name: String::new(),
            minecraft_version: String::new(),
            isolation: IsolationMode::Isolated,
            loader: None,
        }
    }
}

impl InstanceManifest {
    /// Creates a manifest for a new instance whose directory ID equals the
    /// installed Minecraft version.
    pub fn new(id: &VersionId, isolation: IsolationMode) -> Self {
        Self {
            schema_version: INSTANCE_MANIFEST_SCHEMA_VERSION,
            name: id.as_str().to_owned(),
            minecraft_version: id.as_str().to_owned(),
            isolation,
            loader: None,
        }
    }

    pub fn display_name<'a>(&'a self, id: &'a VersionId) -> &'a str {
        let name = self.name.trim();
        if name.is_empty() { id.as_str() } else { name }
    }

    pub fn resolved_minecraft_version<'a>(&'a self, id: &'a VersionId) -> &'a str {
        let version = self.minecraft_version.trim();
        if version.is_empty() {
            id.as_str()
        } else {
            version
        }
    }
}

/// Paths and content-directory semantics for one instance.
///
/// Shared repository files (`libraries/`, `assets/`, `versions/`) stay under
/// [`MinecraftLayout`]. Only the game directory and its content directories move
/// into the instance root when isolation is enabled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceLayout {
    repository: MinecraftLayout,
    id: VersionId,
    isolation: IsolationMode,
}

impl InstanceLayout {
    pub fn new(repository: MinecraftLayout, id: VersionId, isolation: IsolationMode) -> Self {
        Self {
            repository,
            id,
            isolation,
        }
    }

    pub fn repository(&self) -> &MinecraftLayout {
        &self.repository
    }

    pub fn id(&self) -> &VersionId {
        &self.id
    }

    pub fn isolation(&self) -> IsolationMode {
        self.isolation
    }

    /// Directory holding the version JSON, client JAR, natives and (when
    /// isolated) the game directory.
    pub fn root(&self) -> PathBuf {
        self.repository.version_dir(&self.id)
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.root().join(INSTANCE_MANIFEST_FILE)
    }

    pub fn version_json_path(&self) -> PathBuf {
        self.repository.version_json_path(&self.id)
    }

    pub fn client_jar_path_for(&self, jar_id: Option<&VersionId>) -> PathBuf {
        self.repository.client_jar_path(jar_id.unwrap_or(&self.id))
    }

    pub fn natives_dir(&self, platform: &rmcl_domain::Platform) -> PathBuf {
        self.repository.natives_dir(&self.id, platform)
    }

    /// The process working directory and the root of game-owned content.
    pub fn game_dir(&self) -> PathBuf {
        match self.isolation {
            IsolationMode::Isolated => self.root(),
            IsolationMode::Shared => self.repository.root().to_path_buf(),
        }
    }

    pub fn mods_dir(&self) -> PathBuf {
        self.game_dir().join("mods")
    }

    pub fn resourcepacks_dir(&self) -> PathBuf {
        self.game_dir().join("resourcepacks")
    }

    pub fn shaderpacks_dir(&self) -> PathBuf {
        self.game_dir().join("shaderpacks")
    }

    pub fn saves_dir(&self) -> PathBuf {
        self.game_dir().join("saves")
    }

    pub fn options_path(&self) -> PathBuf {
        self.game_dir().join("options.txt")
    }

    /// Creates the game directory and the standard content directories.
    pub fn ensure_content_dirs(&self) -> Result<(), InstanceError> {
        for path in [
            self.game_dir(),
            self.mods_dir(),
            self.resourcepacks_dir(),
            self.shaderpacks_dir(),
            self.saves_dir(),
        ] {
            std::fs::create_dir_all(&path).map_err(|source| InstanceError::Io {
                operation: "create instance content directory",
                path,
                source,
            })?;
        }
        Ok(())
    }

    /// Loads the RMCL manifest, returning `None` for a legacy version that has
    /// no manifest yet.
    ///
    /// A present but malformed or unsupported manifest is an error rather than
    /// a fallback to shared mode, because guessing would silently move the game
    /// directory of an existing instance.
    pub fn load_manifest(&self) -> Result<Option<InstanceManifest>, InstanceError> {
        let path = self.manifest_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(InstanceError::Io {
                    operation: "read instance manifest",
                    path,
                    source,
                });
            }
        };
        if text.trim().is_empty() {
            return Err(InstanceError::EmptyManifest { path });
        }
        let manifest: InstanceManifest =
            toml::from_str(&text).map_err(|source| InstanceError::Toml {
                path: path.clone(),
                source: Box::new(source),
            })?;
        if manifest.schema_version == 0
            || manifest.schema_version > INSTANCE_MANIFEST_SCHEMA_VERSION
        {
            return Err(InstanceError::UnsupportedSchema {
                found: manifest.schema_version,
                supported: INSTANCE_MANIFEST_SCHEMA_VERSION,
            });
        }
        Ok(Some(manifest))
    }

    /// Atomically writes the manifest through a synced temporary file.
    pub fn write_manifest(&self, manifest: &InstanceManifest) -> Result<(), InstanceError> {
        if manifest.schema_version != INSTANCE_MANIFEST_SCHEMA_VERSION {
            return Err(InstanceError::UnsupportedSchema {
                found: manifest.schema_version,
                supported: INSTANCE_MANIFEST_SCHEMA_VERSION,
            });
        }
        let path = self.manifest_path();
        let parent = path.parent().ok_or_else(|| InstanceError::Io {
            operation: "find instance manifest directory",
            path: path.clone(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "instance manifest path has no parent",
            ),
        })?;
        std::fs::create_dir_all(parent).map_err(|source| InstanceError::Io {
            operation: "create instance directory",
            path: parent.to_path_buf(),
            source,
        })?;
        let text =
            toml::to_string_pretty(manifest).map_err(|source| InstanceError::TomlSerialize {
                path: path.clone(),
                source: Box::new(source),
            })?;
        let mut temporary = NamedTempFile::new_in(parent).map_err(|source| InstanceError::Io {
            operation: "create instance manifest temporary file",
            path: parent.to_path_buf(),
            source,
        })?;
        temporary
            .write_all(text.as_bytes())
            .map_err(|source| InstanceError::Io {
                operation: "write instance manifest temporary file",
                path: temporary.path().to_path_buf(),
                source,
            })?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|source| InstanceError::Io {
                operation: "sync instance manifest temporary file",
                path: temporary.path().to_path_buf(),
                source,
            })?;
        temporary
            .persist(&path)
            .map_err(|error| InstanceError::Io {
                operation: "publish instance manifest",
                path: path.clone(),
                source: error.error,
            })?;
        Ok(())
    }
}

/// Errors from reading, writing or laying out an RMCL instance.
#[derive(Debug, Error)]
pub enum InstanceError {
    #[error("failed to {operation} `{path}`: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("instance manifest `{path}` is empty")]
    EmptyManifest { path: PathBuf },
    #[error("invalid instance manifest `{path}`: {source}")]
    Toml {
        path: PathBuf,
        #[source]
        source: Box<toml::de::Error>,
    },
    #[error("failed to serialize instance manifest `{path}`: {source}")]
    TomlSerialize {
        path: PathBuf,
        #[source]
        source: Box<toml::ser::Error>,
    },
    #[error("unsupported instance manifest schema {found}; this build supports schema {supported}")]
    UnsupportedSchema { found: u32, supported: u32 },
    #[error(transparent)]
    Layout(#[from] LayoutError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcl_domain::{OperatingSystem, Platform};
    use std::path::Path;
    use tempfile::TempDir;

    fn id(value: &str) -> VersionId {
        VersionId::new(value).unwrap()
    }

    fn platform() -> Platform {
        Platform::new(OperatingSystem::MacOs, "15.0", "arm64")
    }

    #[test]
    fn isolated_layout_keeps_shared_files_in_the_repository() {
        let repository = MinecraftLayout::new(".minecraft");
        let instance = repository.instance(id("fabric-1.21.1"), IsolationMode::Isolated);

        assert_eq!(
            instance.root(),
            Path::new(".minecraft/versions/fabric-1.21.1")
        );
        assert_eq!(
            instance.game_dir(),
            Path::new(".minecraft/versions/fabric-1.21.1")
        );
        assert_eq!(
            instance.mods_dir(),
            Path::new(".minecraft/versions/fabric-1.21.1/mods")
        );
        assert_eq!(
            instance.resourcepacks_dir(),
            Path::new(".minecraft/versions/fabric-1.21.1/resourcepacks")
        );
        assert_eq!(
            instance.shaderpacks_dir(),
            Path::new(".minecraft/versions/fabric-1.21.1/shaderpacks")
        );
        assert_eq!(
            instance.natives_dir(&platform()),
            Path::new(".minecraft/versions/fabric-1.21.1/natives-macos-aarch64")
        );
        assert_eq!(
            instance.repository().libraries_dir(),
            Path::new(".minecraft/libraries")
        );
        assert_eq!(
            instance.repository().assets_dir(),
            Path::new(".minecraft/assets")
        );
    }

    #[test]
    fn shared_layout_uses_the_repository_as_the_game_directory() {
        let repository = MinecraftLayout::new(".minecraft");
        let instance = repository.instance(id("1.21.1"), IsolationMode::Shared);

        assert_eq!(instance.root(), Path::new(".minecraft/versions/1.21.1"));
        assert_eq!(instance.game_dir(), Path::new(".minecraft"));
        assert_eq!(instance.mods_dir(), Path::new(".minecraft/mods"));
        assert_eq!(
            instance.manifest_path(),
            Path::new(".minecraft/versions/1.21.1/rmcl.instance.toml")
        );
    }

    #[test]
    fn manifest_round_trips_and_missing_manifest_is_legacy() {
        let temporary = TempDir::new().unwrap();
        let repository = MinecraftLayout::new(temporary.path());
        let instance = repository.instance(id("1.21.1"), IsolationMode::Isolated);

        assert_eq!(instance.load_manifest().unwrap(), None);

        let mut manifest = InstanceManifest::new(&id("1.21.1"), IsolationMode::Isolated);
        manifest.name = "My World".to_owned();
        manifest.minecraft_version = "1.21.1".to_owned();
        manifest.loader = Some(InstanceLoader {
            kind: LoaderKind::Fabric,
            version: "0.16.10".to_owned(),
        });

        instance.write_manifest(&manifest).unwrap();

        assert_eq!(instance.load_manifest().unwrap(), Some(manifest));
    }

    #[test]
    fn empty_manifest_is_rejected_instead_of_defaulting_to_isolated() {
        let temporary = TempDir::new().unwrap();
        let repository = MinecraftLayout::new(temporary.path());
        let instance = repository.instance(id("1.21.1"), IsolationMode::Shared);
        std::fs::create_dir_all(instance.root()).unwrap();
        std::fs::write(instance.manifest_path(), b"  \n").unwrap();

        assert!(matches!(
            instance.load_manifest(),
            Err(InstanceError::EmptyManifest { .. })
        ));
    }

    #[test]
    fn unsupported_manifest_schema_is_an_error() {
        let temporary = TempDir::new().unwrap();
        let repository = MinecraftLayout::new(temporary.path());
        let instance = repository.instance(id("1.21.1"), IsolationMode::Isolated);
        std::fs::create_dir_all(instance.root()).unwrap();
        std::fs::write(
            instance.manifest_path(),
            "schema_version = 99\nname = \"future\"\n",
        )
        .unwrap();

        assert!(matches!(
            instance.load_manifest(),
            Err(InstanceError::UnsupportedSchema {
                found: 99,
                supported: INSTANCE_MANIFEST_SCHEMA_VERSION
            })
        ));
    }

    #[test]
    fn blank_manifest_fields_fall_back_to_the_directory_id() {
        let manifest = InstanceManifest::default();
        let id = id("1.21.1");

        assert_eq!(manifest.display_name(&id), "1.21.1");
        assert_eq!(manifest.resolved_minecraft_version(&id), "1.21.1");
    }

    #[test]
    fn content_directories_are_created_under_the_instance_root() {
        let temporary = TempDir::new().unwrap();
        let repository = MinecraftLayout::new(temporary.path());
        let instance = repository.instance(id("1.21.1"), IsolationMode::Isolated);

        instance.ensure_content_dirs().unwrap();

        assert!(instance.mods_dir().is_dir());
        assert!(instance.resourcepacks_dir().is_dir());
        assert!(instance.shaderpacks_dir().is_dir());
        assert!(instance.saves_dir().is_dir());
    }
}
