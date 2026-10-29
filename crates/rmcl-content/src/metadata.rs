//! Local jar metadata used for dependency preflight and recommendations.
//!
//! RMCL reads only well-known metadata entries from mod archives. It does not
//! load classes or execute mod code.

use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use rmcl_instance::{InstanceLayout, RepositoryPath};
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;
use zip::ZipArchive;

use crate::{ContentKind, local::scan_kind};

const MAX_METADATA_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModMetadataLoader {
    Fabric,
    Quilt,
    Forge,
    NeoForge,
    LegacyForge,
}

impl ModMetadataLoader {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fabric => "Fabric",
            Self::Quilt => "Quilt",
            Self::Forge => "Forge",
            Self::NeoForge => "NeoForge",
            Self::LegacyForge => "Legacy Forge",
        }
    }

    pub const fn version_style(self) -> crate::VersionStyle {
        match self {
            Self::Fabric | Self::Quilt => crate::VersionStyle::Semver,
            Self::Forge | Self::NeoForge | Self::LegacyForge => crate::VersionStyle::Maven,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModDependency {
    pub id: String,
    pub version_requirement: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModMetadata {
    pub id: String,
    pub version: Option<String>,
    pub name: Option<String>,
    pub loader: ModMetadataLoader,
    pub dependencies: Vec<ModDependency>,
    /// Declared breaks/conflicts, optionally limited to matching installed versions.
    pub conflicts: Vec<ModDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledMod {
    pub metadata: ModMetadata,
    pub relative_path: RepositoryPath,
    pub file_name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstalledModIndex {
    pub mods: Vec<InstalledMod>,
    pub warnings: Vec<String>,
}

impl InstalledModIndex {
    pub fn find(&self, id: &str) -> Option<&InstalledMod> {
        self.mods
            .iter()
            .find(|installed| installed.metadata.id.eq_ignore_ascii_case(id))
    }

    pub fn has_mod(&self, id: &str) -> bool {
        self.find(id).is_some()
    }

    pub fn has_enabled_mod(&self, id: &str) -> bool {
        self.find(id).is_some_and(|installed| installed.enabled)
    }

    pub fn has_shader_loader(&self) -> bool {
        ["iris", "oculus", "optifine"]
            .iter()
            .any(|id| self.has_mod(id))
    }

    pub fn shader_loader_name(&self) -> Option<&'static str> {
        if self.has_mod("iris") {
            Some("Iris")
        } else if self.has_mod("oculus") {
            Some("Oculus")
        } else if self.has_mod("optifine") {
            Some("OptiFine")
        } else {
            None
        }
    }
}

/// Scans every mod jar in one instance and parses its metadata.
///
/// `.disabled` mods are included so preflight can report an installed-but-
/// disabled dependency instead of silently treating it as available.
pub fn scan_installed_mods(instance: &InstanceLayout) -> InstalledModIndex {
    let scan = scan_kind(instance, ContentKind::Mod);
    let mut index = InstalledModIndex {
        mods: Vec::new(),
        warnings: scan
            .warnings
            .into_iter()
            .map(|warning| warning.message())
            .collect(),
    };
    for item in scan.items {
        if item.is_directory {
            continue;
        }
        let relative_path = item.relative_path.clone();
        let file_name = item.file_name().to_owned();
        let enabled = item.enabled;
        match read_mod_metadata(&item.path) {
            Ok(metadata) => {
                for metadata in metadata {
                    index.mods.push(InstalledMod {
                        metadata,
                        relative_path: relative_path.clone(),
                        file_name: file_name.clone(),
                        enabled,
                    });
                }
            }
            Err(error) => index
                .warnings
                .push(format!("无法读取 {}：{error}", item.display_name())),
        }
    }
    index.warnings.extend(duplicate_mod_warnings(&index.mods));
    index
}

fn duplicate_mod_warnings(mods: &[InstalledMod]) -> Vec<String> {
    let mut by_id: BTreeMap<String, Vec<&InstalledMod>> = BTreeMap::new();
    for installed in mods {
        by_id
            .entry(installed.metadata.id.to_ascii_lowercase())
            .or_default()
            .push(installed);
    }
    by_id
        .into_values()
        .filter(|entries| entries.len() > 1)
        .map(|entries| {
            let id = &entries[0].metadata.id;
            let files = entries
                .iter()
                .map(|entry| entry.file_name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!("检测到重复 Mod ID `{id}`：{files}")
        })
        .collect()
}

/// Reads Fabric, Quilt, Forge, NeoForge, or legacy Forge metadata from a JAR.
pub fn read_mod_metadata(path: &Path) -> Result<Vec<ModMetadata>, ModMetadataError> {
    let file = File::open(path).map_err(|source| ModMetadataError::Open {
        path: path.to_path_buf(),
        source,
    })?;
    let mut archive = ZipArchive::new(file).map_err(|source| ModMetadataError::Zip {
        path: path.to_path_buf(),
        source,
    })?;

    if let Some(bytes) = read_entry(&mut archive, "META-INF/neoforge.mods.toml")? {
        return parse_forge_toml(path, &bytes, ModMetadataLoader::NeoForge);
    }
    if let Some(bytes) = read_entry(&mut archive, "META-INF/mods.toml")? {
        return parse_forge_toml(path, &bytes, ModMetadataLoader::Forge);
    }
    if let Some(bytes) = read_entry(&mut archive, "quilt.mod.json")? {
        return parse_quilt_json(path, &bytes);
    }
    if let Some(bytes) = read_entry(&mut archive, "fabric.mod.json")? {
        return parse_fabric_json(path, &bytes);
    }
    if let Some(bytes) = read_entry(&mut archive, "mcmod.info")? {
        return parse_legacy_json(path, &bytes);
    }
    Err(ModMetadataError::Missing {
        path: path.to_path_buf(),
    })
}

fn read_entry(
    archive: &mut ZipArchive<File>,
    entry: &str,
) -> Result<Option<Vec<u8>>, ModMetadataError> {
    let file = match archive.by_name(entry) {
        Ok(file) => file,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(source) => {
            return Err(ModMetadataError::Zip {
                path: PathBuf::from(entry),
                source,
            });
        }
    };
    if file.size() > MAX_METADATA_BYTES {
        return Err(ModMetadataError::TooLarge {
            entry: entry.to_owned(),
        });
    }
    let mut bytes = Vec::with_capacity(file.size() as usize);
    file.take(MAX_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| ModMetadataError::Read {
            entry: entry.to_owned(),
            source,
        })?;
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        return Err(ModMetadataError::TooLarge {
            entry: entry.to_owned(),
        });
    }
    Ok(Some(bytes))
}

fn parse_fabric_json(path: &Path, bytes: &[u8]) -> Result<Vec<ModMetadata>, ModMetadataError> {
    #[derive(Deserialize)]
    struct FabricFile {
        id: String,
        #[serde(default)]
        version: Option<String>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        depends: BTreeMap<String, Value>,
        #[serde(default)]
        breaks: BTreeMap<String, Value>,
        #[serde(default)]
        conflicts: BTreeMap<String, Value>,
    }

    let file: FabricFile =
        serde_json::from_slice(bytes).map_err(|source| ModMetadataError::Json {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(vec![ModMetadata {
        id: file.id,
        version: file.version,
        name: file.name,
        loader: ModMetadataLoader::Fabric,
        dependencies: value_map_to_dependencies(file.depends),
        conflicts: value_map_to_dependencies(file.breaks)
            .into_iter()
            .chain(value_map_to_dependencies(file.conflicts))
            .collect(),
    }])
}

fn parse_quilt_json(path: &Path, bytes: &[u8]) -> Result<Vec<ModMetadata>, ModMetadataError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|source| ModMetadataError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    let Some(loader) = value.get("quilt_loader") else {
        return Err(ModMetadataError::MissingQuiltLoader {
            path: path.to_path_buf(),
        });
    };
    let id = loader
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| ModMetadataError::Invalid {
            path: path.to_path_buf(),
            reason: "quilt_loader.id is missing".to_owned(),
        })?
        .to_owned();
    let version = loader
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let name = value
        .get("metadata")
        .and_then(|metadata| metadata.get("name"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let dependencies = loader
        .get("depends")
        .map(parse_quilt_dependencies)
        .unwrap_or_default();
    let conflicts = loader
        .get("breaks")
        .map(parse_quilt_dependencies)
        .unwrap_or_default();
    Ok(vec![ModMetadata {
        id,
        version,
        name,
        loader: ModMetadataLoader::Quilt,
        dependencies,
        conflicts,
    }])
}

fn parse_quilt_dependencies(value: &Value) -> Vec<ModDependency> {
    let mut dependencies = Vec::new();
    match value {
        Value::Array(entries) => {
            for entry in entries {
                let Some(id) = entry.get("id").and_then(Value::as_str) else {
                    continue;
                };
                dependencies.push(ModDependency {
                    id: id.to_owned(),
                    version_requirement: entry.get("versions").and_then(format_value_requirement),
                });
            }
        }
        Value::Object(entries) => {
            for (id, requirement) in entries {
                dependencies.push(ModDependency {
                    id: id.clone(),
                    version_requirement: format_value_requirement(requirement),
                });
            }
        }
        _ => {}
    }
    dependencies
}

fn value_map_to_dependencies(map: BTreeMap<String, Value>) -> Vec<ModDependency> {
    map.into_iter()
        .map(|(id, requirement)| ModDependency {
            id,
            version_requirement: format_value_requirement(&requirement),
        })
        .collect()
}

fn format_value_requirement(value: &Value) -> Option<String> {
    match value {
        Value::String(requirement) => Some(requirement.clone()),
        Value::Array(requirements) => {
            let values = requirements
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            (!values.is_empty()).then(|| values.join(", "))
        }
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

fn parse_forge_toml(
    path: &Path,
    bytes: &[u8],
    loader: ModMetadataLoader,
) -> Result<Vec<ModMetadata>, ModMetadataError> {
    #[derive(Deserialize)]
    struct ForgeFile {
        #[serde(default)]
        mods: Vec<ForgeMod>,
        #[serde(default)]
        dependencies: BTreeMap<String, Vec<ForgeDependency>>,
    }

    #[derive(Deserialize)]
    struct ForgeMod {
        #[serde(rename = "modId")]
        mod_id: String,
        #[serde(default)]
        version: Option<String>,
        #[serde(rename = "displayName", default)]
        display_name: Option<String>,
    }

    #[derive(Deserialize)]
    struct ForgeDependency {
        #[serde(rename = "modId")]
        mod_id: String,
        #[serde(default)]
        mandatory: Option<bool>,
        #[serde(rename = "versionRange", default)]
        version_range: Option<String>,
        #[serde(rename = "type", default)]
        dependency_type: Option<String>,
    }

    let text = std::str::from_utf8(bytes).map_err(|source| ModMetadataError::Invalid {
        path: path.to_path_buf(),
        reason: format!("metadata is not UTF-8: {source}"),
    })?;
    let file: ForgeFile = toml::from_str(text).map_err(|source| ModMetadataError::Toml {
        path: path.to_path_buf(),
        source,
    })?;
    if file.mods.is_empty() {
        return Err(ModMetadataError::Invalid {
            path: path.to_path_buf(),
            reason: "no [[mods]] entries".to_owned(),
        });
    }

    Ok(file
        .mods
        .into_iter()
        .map(|forge_mod| {
            let mut dependencies = Vec::new();
            let mut conflicts = Vec::new();
            for dependency in file
                .dependencies
                .get(&forge_mod.mod_id)
                .into_iter()
                .flatten()
            {
                let required = dependency.mandatory.unwrap_or(true)
                    && dependency.dependency_type.as_deref() != Some("optional");
                let incompatible = dependency.dependency_type.as_deref() == Some("incompatible");
                let parsed = ModDependency {
                    id: dependency.mod_id.clone(),
                    version_requirement: dependency.version_range.clone(),
                };
                if incompatible {
                    conflicts.push(parsed);
                } else if required {
                    dependencies.push(parsed);
                }
            }
            ModMetadata {
                id: forge_mod.mod_id,
                version: forge_mod.version,
                name: forge_mod.display_name,
                loader,
                dependencies,
                conflicts,
            }
        })
        .collect())
}

fn parse_legacy_json(path: &Path, bytes: &[u8]) -> Result<Vec<ModMetadata>, ModMetadataError> {
    #[derive(Deserialize)]
    struct LegacyMod {
        modid: String,
        #[serde(default)]
        version: Option<String>,
        #[serde(default)]
        name: Option<String>,
        #[serde(rename = "requiredMods", default)]
        required_mods: Vec<String>,
    }

    let value: Value = serde_json::from_slice(bytes).map_err(|source| ModMetadataError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    let entries = value
        .as_array()
        .cloned()
        .or_else(|| value.get("modList").and_then(Value::as_array).cloned())
        .ok_or_else(|| ModMetadataError::Invalid {
            path: path.to_path_buf(),
            reason: "mcmod.info is neither an array nor an object with modList".to_owned(),
        })?;
    entries
        .into_iter()
        .map(|entry| {
            let legacy: LegacyMod =
                serde_json::from_value(entry).map_err(|source| ModMetadataError::Json {
                    path: path.to_path_buf(),
                    source,
                })?;
            Ok(ModMetadata {
                id: legacy.modid,
                version: legacy.version,
                name: legacy.name,
                loader: ModMetadataLoader::LegacyForge,
                dependencies: legacy
                    .required_mods
                    .into_iter()
                    .map(|id| ModDependency {
                        id,
                        version_requirement: None,
                    })
                    .collect(),
                conflicts: Vec::new(),
            })
        })
        .collect()
}

#[derive(Debug, Error)]
pub enum ModMetadataError {
    #[error("failed to open mod archive `{path}`: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid mod archive `{path}`: {source}")]
    Zip {
        path: PathBuf,
        #[source]
        source: zip::result::ZipError,
    },
    #[error("failed to read mod metadata entry `{entry}`: {source}")]
    Read {
        entry: String,
        #[source]
        source: std::io::Error,
    },
    #[error("mod metadata entry `{entry}` exceeds the size limit")]
    TooLarge { entry: String },
    #[error("mod archive `{path}` has no supported metadata")]
    Missing { path: PathBuf },
    #[error("quilt metadata in `{path}` has no quilt_loader")]
    MissingQuiltLoader { path: PathBuf },
    #[error("invalid mod metadata in `{path}`: {reason}")]
    Invalid { path: PathBuf, reason: String },
    #[error("invalid JSON metadata in `{path}`: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid TOML metadata in `{path}`: {source}")]
    Toml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::TempDir;
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    use super::*;

    fn jar(entries: &[(&str, &str)]) -> (TempDir, PathBuf) {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("mod.jar");
        let file = File::create(&path).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, contents) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(contents.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        (temporary, path)
    }

    #[test]
    fn parses_fabric_metadata_and_dependencies() {
        let (_temporary, path) = jar(&[(
            "fabric.mod.json",
            r#"{
                "id": "example",
                "version": "1.2.3",
                "name": "Example",
                "depends": {"fabric-api": ">=0.92.0", "minecraft": "1.21.x"},
                "breaks": {"bad-mod": "*"}
            }"#,
        )]);
        let metadata = read_mod_metadata(&path).unwrap();
        assert_eq!(metadata.len(), 1);
        assert_eq!(metadata[0].id, "example");
        assert_eq!(metadata[0].loader, ModMetadataLoader::Fabric);
        assert_eq!(metadata[0].dependencies.len(), 2);
        assert_eq!(metadata[0].dependencies[0].id, "fabric-api");
        assert_eq!(metadata[0].conflicts[0].id, "bad-mod");
    }

    #[test]
    fn parses_forge_metadata_and_required_dependencies() {
        let (_temporary, path) = jar(&[(
            "META-INF/mods.toml",
            r#"modLoader = "javafml"
loaderVersion = "[47,)"
license = "MIT"

[[mods]]
modId = "example"
version = "2.0.0"
displayName = "Example"

[[dependencies.example]]
modId = "forge"
mandatory = true
versionRange = "[47,)"
ordering = "NONE"
side = "BOTH"

[[dependencies.example]]
modId = "other"
mandatory = false
versionRange = "[1.0,)"
ordering = "NONE"
side = "BOTH"
"#,
        )]);
        let metadata = read_mod_metadata(&path).unwrap();
        assert_eq!(metadata[0].id, "example");
        assert_eq!(metadata[0].loader, ModMetadataLoader::Forge);
        assert_eq!(metadata[0].dependencies.len(), 1);
        assert_eq!(metadata[0].dependencies[0].id, "forge");
    }

    #[test]
    fn parses_quilt_metadata() {
        let (_temporary, path) = jar(&[(
            "quilt.mod.json",
            r#"{
                "quilt_loader": {
                    "id": "example",
                    "version": "1.0.0",
                    "depends": [
                        {"id": "quilt_loader", "versions": ">=0.20.0"},
                        {"id": "fabric-api", "versions": [">=0.90.0", "<1.0.0"]}
                    ]
                },
                "metadata": {"name": "Example"}
            }"#,
        )]);
        let metadata = read_mod_metadata(&path).unwrap();
        assert_eq!(metadata[0].loader, ModMetadataLoader::Quilt);
        assert_eq!(metadata[0].dependencies.len(), 2);
        assert_eq!(
            metadata[0].dependencies[1].version_requirement.as_deref(),
            Some(">=0.90.0, <1.0.0")
        );
    }

    #[test]
    fn missing_metadata_is_reported() {
        let (_temporary, path) = jar(&[("README.md", "not a mod")]);
        assert!(matches!(
            read_mod_metadata(&path),
            Err(ModMetadataError::Missing { .. })
        ));
    }

    #[test]
    fn scans_installed_mods_and_reports_duplicate_ids() {
        use rmcl_instance::{IsolationMode, MinecraftLayout, VersionId};

        let temporary = TempDir::new().unwrap();
        let instance = MinecraftLayout::new(temporary.path())
            .instance(VersionId::new("test").unwrap(), IsolationMode::Isolated);
        let mods = instance.mods_dir();
        std::fs::create_dir_all(&mods).unwrap();
        let bytes = jar_bytes("example");
        for name in ["one.jar", "two.jar.disabled"] {
            std::fs::write(mods.join(name), &bytes).unwrap();
        }

        let index = scan_installed_mods(&instance);
        assert_eq!(index.mods.len(), 2);
        assert!(index.mods.iter().any(|installed| !installed.enabled));
        assert!(
            index
                .warnings
                .iter()
                .any(|warning| warning.contains("重复 Mod ID"))
        );
        assert!(index.has_mod("example"));
    }

    fn jar_bytes(id: &str) -> Vec<u8> {
        let (temporary, path) = jar(&[(
            "fabric.mod.json",
            &format!(r#"{{"id":"{id}","version":"1.0.0"}}"#),
        )]);
        let bytes = std::fs::read(&path).unwrap();
        drop(temporary);
        bytes
    }
}
