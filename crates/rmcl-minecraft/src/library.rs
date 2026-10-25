use std::collections::BTreeMap;

use serde::Deserialize;

use crate::{DownloadInfo, Rule};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Library {
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
    /// Fabric/Quilt launcher profiles put integrity fields at library level.
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub downloads: Option<LibraryDownloads>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub natives: BTreeMap<String, String>,
    #[serde(default)]
    pub extract: Option<ExtractRules>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct LibraryDownloads {
    #[serde(default)]
    pub artifact: Option<DownloadInfo>,
    #[serde(default)]
    pub classifiers: BTreeMap<String, DownloadInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ExtractRules {
    #[serde(default)]
    pub exclude: Vec<String>,
}

impl ExtractRules {
    pub fn allows(&self, archive_path: &str) -> bool {
        !self
            .exclude
            .iter()
            .any(|prefix| archive_path.starts_with(prefix))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extraction_exclusions_use_archive_path_prefixes() {
        let rules = ExtractRules {
            exclude: vec!["META-INF/".to_owned(), "native/".to_owned()],
        };

        assert!(!rules.allows("META-INF/MANIFEST.MF"));
        assert!(!rules.allows("native/image.png"));
        assert!(rules.allows("org/lwjgl/SomeClass.class"));
        assert!(rules.allows("other/META-INF/file"));
    }
}
