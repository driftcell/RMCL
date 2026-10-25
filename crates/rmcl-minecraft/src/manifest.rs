use serde::Deserialize;

use crate::MetadataError;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct VersionManifest {
    #[serde(default)]
    pub latest: Option<LatestVersions>,
    pub versions: Vec<VersionSummary>,
}

impl VersionManifest {
    pub fn from_json(json: &str) -> Result<Self, MetadataError> {
        Ok(serde_json::from_str(json)?)
    }

    pub fn find(&self, id: &str) -> Option<&VersionSummary> {
        self.versions.iter().find(|version| version.id == id)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct LatestVersions {
    #[serde(default)]
    pub release: Option<String>,
    #[serde(default)]
    pub snapshot: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionSummary {
    pub id: String,
    #[serde(rename = "type", default)]
    pub version_type: Option<String>,
    pub url: String,
    #[serde(default)]
    pub time: Option<String>,
    #[serde(default)]
    pub release_time: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_remote_manifest_and_finds_exact_id() {
        let manifest = VersionManifest::from_json(
            r#"{
                "latest": {"release": "1.21.1", "snapshot": "24w99a"},
                "versions": [
                    {
                        "id": "1.21.1",
                        "type": "release",
                        "url": "https://example.invalid/1.21.1.json",
                        "time": "2024-08-08T08:00:00+00:00",
                        "releaseTime": "2024-08-08T08:00:00+00:00"
                    },
                    {"id": "local", "url": "https://example.invalid/local.json"}
                ]
            }"#,
        )
        .unwrap();

        assert_eq!(manifest.versions.len(), 2);
        assert_eq!(
            manifest.find("1.21.1").unwrap().url,
            "https://example.invalid/1.21.1.json"
        );
        assert_eq!(manifest.find("local").unwrap().version_type, None);
        assert!(manifest.find("1.21").is_none());
        assert_eq!(manifest.latest.unwrap().release.as_deref(), Some("1.21.1"));
    }
}
