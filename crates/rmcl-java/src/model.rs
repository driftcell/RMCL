use std::path::PathBuf;

use rmcl_domain::{Architecture, OperatingSystem, Platform};
use rmcl_minecraft::GameVersion;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaRuntime {
    pub executable: PathBuf,
    pub version: String,
    pub major_version: u32,
    pub operating_system: OperatingSystem,
    pub architecture: Architecture,
    pub vendor: Option<String>,
    pub source: JavaRuntimeSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum JavaRuntimeSource {
    Managed,
    JavaHome,
    System,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaRequirement {
    pub component: Option<String>,
    pub major_version: Option<u32>,
    pub maximum_major: Option<u32>,
    pub operating_system: OperatingSystem,
    pub architecture: Architecture,
}

impl JavaRequirement {
    pub fn for_version(version: &GameVersion, platform: &Platform) -> Self {
        let java = version.java_version.as_ref();
        let maximum_major = if platform.os == OperatingSystem::Linux
            && platform.arch == Architecture::X86_64
            && is_legacy_game_version(&version.id)
        {
            Some(8)
        } else {
            None
        };
        Self {
            component: java.and_then(|value| value.component.clone()),
            major_version: java
                .map(|value| value.major_version)
                .or_else(|| default_java_major(&version.id)),
            maximum_major,
            operating_system: platform.os,
            architecture: platform.arch.clone(),
        }
    }

    pub fn accepts(&self, runtime: &JavaRuntime) -> bool {
        let minimum = self.major_version.unwrap_or(8);
        let maximum = self.maximum_major.unwrap_or(u32::MAX);
        self.operating_system == runtime.operating_system
            && self.architecture == runtime.architecture
            && runtime.major_version >= minimum
            && runtime.major_version <= maximum
    }
}

fn default_java_major(id: &str) -> Option<u32> {
    let mut parts = id.split('.');
    let major = parts.next().and_then(parse_component)?;
    let minor = parts.next().and_then(parse_component).unwrap_or(0);
    let patch = parts.next().and_then(parse_component).unwrap_or(0);
    if major >= 26 {
        Some(25)
    } else if major == 1 && (minor > 20 || (minor == 20 && patch >= 5)) {
        Some(21)
    } else if major == 1 && (18..=20).contains(&minor) {
        Some(17)
    } else if major == 1 && minor == 17 {
        Some(16)
    } else if major == 1 && minor <= 16 {
        Some(8)
    } else {
        None
    }
}

fn is_legacy_game_version(id: &str) -> bool {
    let mut parts = id.split('.');
    if parts.next().and_then(parse_component) != Some(1) {
        return false;
    }
    parts
        .next()
        .and_then(parse_component)
        .is_some_and(|minor| minor < 13)
}

fn parse_component(component: &str) -> Option<u32> {
    let digits: String = component.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_version_detection_is_limited_to_pre_1_13() {
        for id in ["1.7.10", "1.12.2", "1.12.2-pre"] {
            assert!(is_legacy_game_version(id), "failed for {id}");
        }
        for id in ["1.13", "1.20.1", "26.3", "unknown"] {
            assert!(!is_legacy_game_version(id), "failed for {id}");
        }
    }

    #[test]
    fn default_major_follows_the_game_generation() {
        for (id, expected) in [
            ("26.3", Some(25)),
            ("1.20.5", Some(21)),
            ("1.20.1", Some(17)),
            ("1.18.2", Some(17)),
            ("1.17.1", Some(16)),
            ("1.16.5", Some(8)),
            ("1.7.10", Some(8)),
            ("unknown", None),
        ] {
            assert_eq!(default_java_major(id), expected, "failed for {id}");
        }
    }

    #[test]
    fn linux_x64_legacy_versions_are_capped_to_java_8() {
        let version = GameVersion::from_json(
            r#"{"id":"1.7.10","javaVersion":{"component":"jre-legacy","majorVersion":8}}"#,
        )
        .unwrap();
        let platform = Platform::new(OperatingSystem::Linux, "6.1", "x86_64");

        let requirement = JavaRequirement::for_version(&version, &platform);

        assert_eq!(component(&requirement), "jre-legacy");
        assert_eq!(requirement.maximum_major, Some(8));
    }

    fn component(requirement: &JavaRequirement) -> &str {
        requirement.component.as_deref().unwrap()
    }
}
