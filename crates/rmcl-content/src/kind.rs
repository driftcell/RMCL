use std::{fmt, path::PathBuf};

use rmcl_instance::InstanceLayout;
use serde::{Deserialize, Serialize};

/// A kind of locally managed Minecraft content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    Mod,
    ResourcePack,
    ShaderPack,
}

impl ContentKind {
    pub const ALL: [Self; 3] = [Self::Mod, Self::ResourcePack, Self::ShaderPack];

    pub const fn directory_name(self) -> &'static str {
        match self {
            Self::Mod => "mods",
            Self::ResourcePack => "resourcepacks",
            Self::ShaderPack => "shaderpacks",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Mod => "Mod",
            Self::ResourcePack => "资源包",
            Self::ShaderPack => "光影包",
        }
    }

    /// Modrinth project type used by search facets and project page URLs.
    pub const fn modrinth_project_type(self) -> &'static str {
        match self {
            Self::Mod => "mod",
            Self::ResourcePack => "resourcepack",
            Self::ShaderPack => "shader",
        }
    }

    pub fn directory(self, instance: &InstanceLayout) -> PathBuf {
        match self {
            Self::Mod => instance.mods_dir(),
            Self::ResourcePack => instance.resourcepacks_dir(),
            Self::ShaderPack => instance.shaderpacks_dir(),
        }
    }

    /// Whether a file name is a supported content file for this kind.
    ///
    /// A trailing `.disabled` suffix is accepted for mods so that already
    /// disabled files remain visible and can be re-enabled.
    pub fn accepts_file_name(self, file_name: &str) -> bool {
        let (base, disabled) = strip_disabled_suffix(file_name);
        if disabled && self != Self::Mod {
            return false;
        }
        let Some((_, extension)) = base.rsplit_once('.') else {
            return false;
        };
        let extension = extension.to_ascii_lowercase();
        match self {
            Self::Mod => matches!(extension.as_str(), "jar" | "zip" | "litemod"),
            Self::ResourcePack | Self::ShaderPack => extension == "zip",
        }
    }

    /// Resource and shader packs may be directories; mods are always files.
    pub const fn accepts_directory(self) -> bool {
        matches!(self, Self::ResourcePack | Self::ShaderPack)
    }
}

impl fmt::Display for ContentKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

pub(crate) fn strip_disabled_suffix(file_name: &str) -> (&str, bool) {
    match file_name.strip_suffix(".disabled") {
        Some(base) if !base.is_empty() => (base, true),
        _ => (file_name, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_mods_accept_the_disabled_suffix() {
        assert!(ContentKind::Mod.accepts_file_name("example.jar"));
        assert!(ContentKind::Mod.accepts_file_name("example.jar.disabled"));
        assert!(ContentKind::Mod.accepts_file_name("example.zip"));
        assert!(ContentKind::Mod.accepts_file_name("example.litemod"));
        assert!(!ContentKind::ResourcePack.accepts_file_name("pack.zip.disabled"));
        assert!(!ContentKind::ShaderPack.accepts_file_name("shader.zip.disabled"));
        assert!(!ContentKind::Mod.accepts_file_name("notes.txt"));
    }
}
