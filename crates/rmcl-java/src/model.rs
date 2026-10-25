use std::path::PathBuf;

use rmcl_domain::Architecture;
use rmcl_minecraft::GameVersion;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaRuntime {
    pub executable: PathBuf,
    pub version: String,
    pub major_version: u32,
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
    pub architecture: Architecture,
}

impl JavaRequirement {
    pub fn for_version(version: &GameVersion, architecture: Architecture) -> Self {
        let java = version.java_version.as_ref();
        Self {
            component: java.and_then(|value| value.component.clone()),
            major_version: java.map(|value| value.major_version),
            architecture,
        }
    }

    pub fn accepts(&self, runtime: &JavaRuntime) -> bool {
        let baseline = self.major_version.unwrap_or(8);
        self.architecture == runtime.architecture
            && runtime.major_version >= baseline
            && self
                .major_version
                .is_none_or(|required| required == runtime.major_version)
    }
}
