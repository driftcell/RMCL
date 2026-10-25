use std::collections::BTreeMap;

use serde::Deserialize;

use crate::MetadataError;

/// The object catalog contained in a downloaded asset index.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct AssetCatalog {
    #[serde(default)]
    pub objects: BTreeMap<String, AssetObject>,
    #[serde(rename = "virtual", default)]
    pub is_virtual: Option<bool>,
    #[serde(default)]
    pub map_to_resources: Option<bool>,
}

impl AssetCatalog {
    pub fn from_json(json: &str) -> Result<Self, MetadataError> {
        Ok(serde_json::from_str(json)?)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_asset_objects_and_legacy_flags() {
        let catalog = AssetCatalog::from_json(
            r#"{
                "virtual": true,
                "map_to_resources": true,
                "objects": {
                    "icons/icon.png": {
                        "hash": "0123456789abcdef0123456789abcdef01234567",
                        "size": 42
                    }
                }
            }"#,
        )
        .unwrap();

        assert_eq!(catalog.is_virtual, Some(true));
        assert_eq!(catalog.map_to_resources, Some(true));
        assert_eq!(catalog.objects["icons/icon.png"].size, 42);
    }

    #[test]
    fn accepts_empty_catalog() {
        let catalog = AssetCatalog::from_json("{}").unwrap();
        assert!(catalog.objects.is_empty());
        assert_eq!(catalog.is_virtual, None);
    }
}
