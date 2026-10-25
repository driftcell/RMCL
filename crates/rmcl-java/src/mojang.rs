use std::collections::BTreeMap;

use rmcl_domain::{Architecture, OperatingSystem};
use rmcl_instance::RepositoryPath;
use serde::{Deserialize, Serialize};

use crate::{JavaError, JavaRequirement, parse_major_version};

pub const MOJANG_RUNTIME_INDEX_URL: &str = "https://piston-meta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

pub type MojangRuntimeIndex = BTreeMap<String, BTreeMap<String, Vec<MojangRuntimeEntry>>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MojangRuntimeEntry {
    pub version: MojangRuntimeVersion,
    pub manifest: MojangDownload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MojangRuntimeVersion {
    pub name: String,
    pub released: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MojangDownload {
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MojangRuntimeManifest {
    pub files: BTreeMap<String, MojangFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MojangFile {
    #[serde(rename = "type")]
    pub kind: MojangFileType,
    #[serde(default)]
    pub downloads: Option<MojangFileDownloads>,
    #[serde(default)]
    pub executable: Option<bool>,
    #[serde(default)]
    pub target: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MojangFileType {
    File,
    Directory,
    Link,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MojangFileDownloads {
    #[serde(default)]
    pub raw: Option<MojangDownload>,
    #[serde(default)]
    pub lzma: Option<MojangDownload>,
}

pub fn mojang_platform_key(
    os: OperatingSystem,
    architecture: &Architecture,
) -> Result<&'static str, JavaError> {
    match (os, architecture) {
        (OperatingSystem::Windows, Architecture::X86) => Ok("windows-x86"),
        (OperatingSystem::Windows, Architecture::X86_64) => Ok("windows-x64"),
        (OperatingSystem::Windows, Architecture::Aarch64) => Ok("windows-arm64"),
        (OperatingSystem::Linux, Architecture::X86) => Ok("linux-i386"),
        (OperatingSystem::Linux, Architecture::X86_64) => Ok("linux"),
        (OperatingSystem::MacOs, Architecture::X86_64) => Ok("mac-os"),
        (OperatingSystem::MacOs, Architecture::Aarch64) => Ok("mac-os-arm64"),
        (os, architecture) => Err(JavaError::UnsupportedPlatform {
            os,
            architecture: architecture.clone(),
        }),
    }
}

#[derive(Debug, Clone)]
pub struct SelectedRuntime {
    pub component: String,
    pub entry: MojangRuntimeEntry,
}

pub fn select_runtime(
    index: &MojangRuntimeIndex,
    requirement: &JavaRequirement,
) -> Result<SelectedRuntime, JavaError> {
    let major = requirement.major_version.ok_or(JavaError::MissingMajor)?;
    let platform = mojang_platform_key(requirement.operating_system, &requirement.architecture)?;
    let mut components = runtime_components(requirement, major)?;
    let platform_components = index.get(platform).ok_or_else(|| JavaError::NoRuntimeFor {
        platform: platform.to_owned(),
        major,
        components: Vec::new(),
    })?;
    components.sort();
    components.dedup();

    for component in &components {
        let Some(entries) = platform_components.get(component) else {
            continue;
        };
        let mut candidates: Vec<_> = entries
            .iter()
            .filter(|entry| parse_major_version(&entry.version.name) == Some(major))
            .collect();
        candidates.sort_by(|left, right| right.version.released.cmp(&left.version.released));
        if let Some(entry) = candidates.first() {
            validate_component(component)?;
            return Ok(SelectedRuntime {
                component: component.clone(),
                entry: (*entry).clone(),
            });
        }
    }

    Err(JavaError::NoRuntimeFor {
        platform: platform.to_owned(),
        major,
        components,
    })
}

pub fn runtime_components(
    requirement: &JavaRequirement,
    major: u32,
) -> Result<Vec<String>, JavaError> {
    let mut components = Vec::new();
    if let Some(component) = &requirement.component {
        validate_component(component)?;
        components.push(component.clone());
    }
    match canonical_component(major) {
        Ok(component) => components.push(component.to_owned()),
        Err(error) if components.is_empty() => return Err(error),
        Err(_) => {}
    }
    if major == 17 {
        components.push("java-runtime-gamma".to_owned());
        components.push("java-runtime-gamma-snapshot".to_owned());
    }
    Ok(components)
}

pub fn select_runtime_entry(
    index: &MojangRuntimeIndex,
    requirement: &JavaRequirement,
) -> Result<MojangRuntimeEntry, JavaError> {
    Ok(select_runtime(index, requirement)?.entry)
}

fn validate_component(component: &str) -> Result<(), JavaError> {
    let path = RepositoryPath::new(component)?;
    if path.as_str().contains('/') {
        return Err(JavaError::InvalidManifest {
            path: component.to_owned(),
            reason: "runtime component must be a single path component".to_owned(),
        });
    }
    Ok(())
}

pub fn canonical_component(major: u32) -> Result<&'static str, JavaError> {
    match major {
        8 => Ok("jre-legacy"),
        16 => Ok("java-runtime-alpha"),
        17 => Ok("java-runtime-beta"),
        21 => Ok("java-runtime-delta"),
        25 => Ok("java-runtime-epsilon"),
        major => Err(JavaError::UnsupportedMajor { major }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcl_domain::Architecture;

    fn requirement(os: OperatingSystem, architecture: Architecture, major: u32) -> JavaRequirement {
        JavaRequirement {
            component: None,
            major_version: Some(major),
            maximum_major: None,
            operating_system: os,
            architecture,
        }
    }

    fn entry(name: &str, released: &str) -> MojangRuntimeEntry {
        MojangRuntimeEntry {
            version: MojangRuntimeVersion {
                name: name.to_owned(),
                released: released.to_owned(),
            },
            manifest: MojangDownload {
                url: "https://piston-meta.mojang.com/manifest.json".to_owned(),
                sha1: None,
                size: None,
            },
        }
    }

    #[test]
    fn maps_supported_mojang_platform_keys() {
        for (os, architecture, expected) in [
            (OperatingSystem::Windows, Architecture::X86, "windows-x86"),
            (
                OperatingSystem::Windows,
                Architecture::X86_64,
                "windows-x64",
            ),
            (
                OperatingSystem::Windows,
                Architecture::Aarch64,
                "windows-arm64",
            ),
            (OperatingSystem::Linux, Architecture::X86, "linux-i386"),
            (OperatingSystem::Linux, Architecture::X86_64, "linux"),
            (OperatingSystem::MacOs, Architecture::X86_64, "mac-os"),
            (
                OperatingSystem::MacOs,
                Architecture::Aarch64,
                "mac-os-arm64",
            ),
        ] {
            assert_eq!(mojang_platform_key(os, &architecture).unwrap(), expected);
        }
        assert!(mojang_platform_key(OperatingSystem::Linux, &Architecture::Aarch64).is_err());
        assert!(mojang_platform_key(OperatingSystem::MacOs, &Architecture::X86).is_err());
    }

    #[test]
    fn selects_exact_major_by_release_date() {
        let index = MojangRuntimeIndex::from([(
            "linux".to_owned(),
            BTreeMap::from([(
                "java-runtime-delta".to_owned(),
                vec![entry("21.0.7", "2026-01-01"), entry("21.0.8", "2026-02-01")],
            )]),
        )]);

        let selected = select_runtime_entry(
            &index,
            &requirement(OperatingSystem::Linux, Architecture::X86_64, 21),
        )
        .unwrap();

        assert_eq!(selected.version.name, "21.0.8");
    }

    #[test]
    fn arm64_java_17_falls_back_to_gamma() {
        let index = MojangRuntimeIndex::from([(
            "mac-os-arm64".to_owned(),
            BTreeMap::from([
                ("java-runtime-beta".to_owned(), Vec::new()),
                (
                    "java-runtime-gamma".to_owned(),
                    vec![entry("17.0.15", "2026-01-01")],
                ),
            ]),
        )]);
        let requirement = requirement(OperatingSystem::MacOs, Architecture::Aarch64, 17);

        let selected = select_runtime_entry(&index, &requirement).unwrap();

        assert_eq!(selected.version.name, "17.0.15");
    }

    #[test]
    fn rejects_unsafe_components() {
        let mut requirement = requirement(OperatingSystem::Linux, Architecture::X86_64, 21);
        requirement.component = Some("../escape".to_owned());
        let index = MojangRuntimeIndex::new();
        let error = select_runtime_entry(&index, &requirement).unwrap_err();
        assert!(matches!(error, JavaError::Path(_)));
    }

    #[test]
    fn does_not_install_a_different_major() {
        let index = MojangRuntimeIndex::from([(
            "linux".to_owned(),
            BTreeMap::from([(
                "java-runtime-delta".to_owned(),
                vec![entry("21.0.7", "2026-01-01")],
            )]),
        )]);

        let error = select_runtime_entry(
            &index,
            &requirement(OperatingSystem::Linux, Architecture::X86_64, 17),
        )
        .unwrap_err();

        assert!(matches!(error, JavaError::NoRuntimeFor { .. }));
    }

    #[test]
    fn parses_manifest_file_types() {
        let manifest: MojangRuntimeManifest = serde_json::from_str(
            r#"{
                "files": {
                    "bin/java": {"type": "file", "executable": true, "downloads": {"raw": {"url": "https://piston-data.mojang.com/java", "sha1": "0123456789abcdef0123456789abcdef01234567", "size": 10}}},
                    "legal": {"type": "directory"},
                    "legal/readme": {"type": "link", "target": "../java.base/readme"}
                }
            }"#,
        )
        .unwrap();

        assert_eq!(manifest.files["bin/java"].kind, MojangFileType::File);
        assert_eq!(manifest.files["legal"].kind, MojangFileType::Directory);
        assert_eq!(manifest.files["legal/readme"].kind, MojangFileType::Link);
    }
}
