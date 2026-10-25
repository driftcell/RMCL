use serde::Deserialize;

/// Download metadata embedded in a Minecraft version or library description.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct DownloadInfo {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
}
