#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatingSystem {
    Windows,
    MacOs,
    Linux,
}

impl OperatingSystem {
    pub fn from_manifest_name(name: &str) -> Option<Self> {
        match name {
            "windows" => Some(Self::Windows),
            "osx" => Some(Self::MacOs),
            "linux" => Some(Self::Linux),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::MacOs => "osx",
            Self::Linux => "linux",
        }
    }

    pub fn from_java_name(name: &str) -> Option<Self> {
        let name = name.trim().to_lowercase();
        if name.starts_with("windows") {
            Some(Self::Windows)
        } else if matches!(name.as_str(), "mac os x" | "macos" | "osx" | "darwin") {
            Some(Self::MacOs)
        } else if name == "linux" {
            Some(Self::Linux)
        } else {
            None
        }
    }

    pub fn native_directory_name(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::MacOs => "macos",
            Self::Linux => "linux",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Architecture {
    X86,
    X86_64,
    Arm32,
    Aarch64,
    Other(String),
}

impl Architecture {
    pub fn from_name(name: &str) -> Self {
        match name {
            "x86" | "i386" | "i486" | "i586" | "i686" => Self::X86,
            "x86_64" | "x86-64" | "amd64" | "x64" => Self::X86_64,
            "arm" | "arm32" | "armv7" | "armv7l" => Self::Arm32,
            "aarch64" | "arm64" => Self::Aarch64,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::X86 => "x86",
            Self::X86_64 => "x86_64",
            Self::Arm32 => "arm",
            Self::Aarch64 => "aarch64",
            Self::Other(name) => name,
        }
    }

    pub fn pointer_width(&self) -> Option<u8> {
        match self {
            Self::X86 | Self::Arm32 => Some(32),
            Self::X86_64 | Self::Aarch64 => Some(64),
            Self::Other(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub os: OperatingSystem,
    pub os_version: String,
    pub arch: Architecture,
}

impl Platform {
    pub fn new(
        os: OperatingSystem,
        os_version: impl Into<String>,
        arch: impl Into<String>,
    ) -> Self {
        let arch = arch.into();
        Self {
            os,
            os_version: os_version.into(),
            arch: Architecture::from_name(&arch),
        }
    }

    pub fn with_architecture(
        os: OperatingSystem,
        os_version: impl Into<String>,
        arch: Architecture,
    ) -> Self {
        Self {
            os,
            os_version: os_version.into(),
            arch,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_architecture_aliases_and_pointer_width() {
        for name in ["x86_64", "amd64", "x64"] {
            let architecture = Architecture::from_name(name);
            assert_eq!(architecture, Architecture::X86_64);
            assert_eq!(architecture.pointer_width(), Some(64));
            assert_eq!(architecture.as_str(), "x86_64");
        }
        assert_eq!(Architecture::from_name("arm64"), Architecture::Aarch64);
        assert_eq!(Architecture::from_name("i686").pointer_width(), Some(32));
        assert_eq!(Architecture::from_name("riscv64").pointer_width(), None);
    }

    #[test]
    fn maps_os_names_for_manifests_and_native_directories() {
        for (name, os, directory) in [
            ("windows", OperatingSystem::Windows, "windows"),
            ("osx", OperatingSystem::MacOs, "macos"),
            ("linux", OperatingSystem::Linux, "linux"),
        ] {
            assert_eq!(OperatingSystem::from_manifest_name(name), Some(os));
            assert_eq!(os.as_str(), name);
            assert_eq!(os.native_directory_name(), directory);
        }
        assert_eq!(OperatingSystem::from_manifest_name("plan9"), None);
        assert_eq!(
            OperatingSystem::from_java_name("Windows 11"),
            Some(OperatingSystem::Windows)
        );
        assert_eq!(
            OperatingSystem::from_java_name("Mac OS X"),
            Some(OperatingSystem::MacOs)
        );
        assert_eq!(
            OperatingSystem::from_java_name("Linux"),
            Some(OperatingSystem::Linux)
        );
        assert_eq!(OperatingSystem::from_java_name("FreeBSD"), None);
    }
}
