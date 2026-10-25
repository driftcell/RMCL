use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, de::Error as _};

use crate::{
    DownloadInfo, FeatureMap, Library, MetadataError, Platform, Rule, RuleError, evaluate_rules,
};

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameVersion {
    pub id: String,
    #[serde(default)]
    pub inherits_from: Option<String>,
    #[serde(default)]
    pub jar: Option<String>,
    #[serde(default)]
    pub main_class: Option<String>,
    #[serde(default)]
    pub arguments: Option<Arguments>,
    #[serde(default)]
    pub minecraft_arguments: Option<String>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default)]
    pub downloads: BTreeMap<String, DownloadInfo>,
    #[serde(default)]
    pub asset_index: Option<AssetIndex>,
    #[serde(default)]
    pub assets: Option<String>,
    #[serde(default)]
    pub java_version: Option<JavaVersion>,
    #[serde(default)]
    pub minimum_launcher_version: Option<u32>,
    #[serde(default)]
    pub compliance_level: Option<u32>,
    #[serde(default)]
    pub time: Option<String>,
    #[serde(default)]
    pub release_time: Option<String>,
    #[serde(rename = "type", default)]
    pub version_type: Option<String>,
}

impl GameVersion {
    pub fn from_json(json: &str) -> Result<Self, MetadataError> {
        let version: Self = serde_json::from_str(json)?;
        if version.id.trim().is_empty() {
            return Err(MetadataError::BlankVersionId);
        }
        Ok(version)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIndex {
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub total_size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaVersion {
    #[serde(default)]
    pub component: Option<String>,
    pub major_version: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<Argument>,
    #[serde(default)]
    pub jvm: Vec<Argument>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Argument {
    Plain(String),
    Ruled(RuledArgument),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuledArgument {
    pub rules: Vec<Rule>,
    pub values: Vec<String>,
}

impl<'de> Deserialize<'de> for RuledArgument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawRuledArgument::deserialize(deserializer)?;
        if raw.value.is_some() && raw.values.is_some() {
            return Err(D::Error::custom(
                "an argument cannot contain both `value` and `values`",
            ));
        }

        let values = raw
            .value
            .or(raw.values)
            .map(OneOrMany::into_vec)
            .ok_or_else(|| D::Error::custom("a ruled argument requires `value` or `values`"))?;
        Ok(Self {
            rules: raw.rules,
            values,
        })
    }
}

#[derive(Deserialize)]
struct RawRuledArgument {
    #[serde(default)]
    rules: Vec<Rule>,
    #[serde(default)]
    value: Option<OneOrMany>,
    #[serde(default)]
    values: Option<OneOrMany>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::One(value) => vec![value],
            Self::Many(values) => values,
        }
    }
}

/// Selects and flattens modern arguments without performing placeholder replacement.
pub fn evaluate_arguments<'a>(
    arguments: &'a [Argument],
    platform: &Platform,
    features: &FeatureMap,
) -> Result<Vec<&'a str>, RuleError> {
    let mut selected = Vec::new();
    for argument in arguments {
        match argument {
            Argument::Plain(value) => selected.push(value.as_str()),
            Argument::Ruled(argument) if evaluate_rules(&argument.rules, platform, features)? => {
                selected.extend(argument.values.iter().map(String::as_str));
            }
            Argument::Ruled(_) => {}
        }
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OperatingSystem;

    #[test]
    fn parses_modern_version_and_library_metadata() {
        let version = GameVersion::from_json(
            r#"{
                "id": "example-release",
                "type": "release",
                "mainClass": "example.Main",
                "assets": "17",
                "assetIndex": {
                    "id": "17",
                    "url": "https://example.invalid/index.json",
                    "sha1": "0123456789abcdef0123456789abcdef01234567",
                    "size": 100,
                    "totalSize": 200
                },
                "javaVersion": {"component": "java-runtime", "majorVersion": 21},
                "downloads": {
                    "client": {
                        "url": "https://example.invalid/client.jar",
                        "sha1": "0123456789abcdef0123456789abcdef01234567",
                        "size": 42
                    }
                },
                "arguments": {
                    "game": ["--username", "${auth_player_name}"],
                    "jvm": [{"rules": [], "value": "-Xmx2G"}]
                },
                "libraries": [{
                    "name": "example.group:example-library:1.0",
                    "downloads": {
                        "artifact": {
                            "path": "example/group/example-library/1.0/example-library-1.0.jar",
                            "url": "https://example.invalid/library.jar"
                        },
                        "classifiers": {
                            "natives-linux": {
                                "path": "example/group/example-library/1.0/example-library-1.0-natives-linux.jar",
                                "url": "https://example.invalid/library-natives.jar"
                            }
                        }
                    },
                    "natives": {"linux": "natives-linux"},
                    "extract": {"exclude": ["META-INF/"]}
                }],
                "futureField": {"ignored": true}
            }"#,
        )
        .unwrap();

        assert_eq!(version.id, "example-release");
        assert_eq!(version.main_class.as_deref(), Some("example.Main"));
        assert_eq!(version.java_version.unwrap().major_version, 21);
        assert_eq!(version.asset_index.unwrap().total_size, Some(200));
        assert_eq!(version.downloads["client"].size, Some(42));
        assert_eq!(version.arguments.unwrap().game.len(), 2);

        let library = &version.libraries[0];
        let downloads = library.downloads.as_ref().unwrap();
        assert!(downloads.artifact.is_some());
        assert!(downloads.classifiers.contains_key("natives-linux"));
        assert_eq!(library.natives["linux"], "natives-linux");
    }

    #[test]
    fn retains_legacy_and_modern_argument_models() {
        let version = GameVersion::from_json(
            r#"{
                "id": "hybrid",
                "minecraftArguments": "--username ${auth_player_name}",
                "arguments": {"game": ["--demo"], "jvm": []}
            }"#,
        )
        .unwrap();

        assert_eq!(
            version.minecraft_arguments.as_deref(),
            Some("--username ${auth_player_name}")
        );
        assert_eq!(version.arguments.unwrap().game.len(), 1);
    }

    #[test]
    fn rejects_blank_or_missing_version_ids() {
        assert!(matches!(
            GameVersion::from_json(r#"{"id": "  "}"#),
            Err(MetadataError::BlankVersionId)
        ));
        assert!(GameVersion::from_json(r#"{"mainClass": "example.Main"}"#).is_err());
    }

    #[test]
    fn evaluates_plain_and_ruled_arguments_in_order() {
        let arguments: Arguments = serde_json::from_str(
            r#"{
                "game": [
                    "--first",
                    {"rules": [], "value": "--second"},
                    {"rules": [{"action": "allow", "features": {"is_demo_user": true}}], "values": ["--demo", "player"]},
                    {"rules": [{"action": "allow", "features": {"is_demo_user": false}}], "value": "--not-demo"}
                ]
            }"#,
        )
        .unwrap();
        let platform = Platform::new(OperatingSystem::Linux, "6.1", "x86_64");

        let selected = evaluate_arguments(&arguments.game, &platform, &FeatureMap::new()).unwrap();
        assert_eq!(selected, ["--first", "--second", "--not-demo"]);

        let features = FeatureMap::from([("is_demo_user".to_owned(), true)]);
        let selected = evaluate_arguments(&arguments.game, &platform, &features).unwrap();
        assert_eq!(selected, ["--first", "--second", "--demo", "player"]);
    }

    #[test]
    fn rejects_ambiguous_or_missing_argument_values() {
        for json in [
            r#"{"rules": []}"#,
            r#"{"rules": [], "value": "one", "values": ["two"]}"#,
        ] {
            assert!(serde_json::from_str::<Argument>(json).is_err());
        }
    }

    #[test]
    fn rejects_non_string_argument_values() {
        assert!(serde_json::from_str::<Argument>("42").is_err());
        assert!(serde_json::from_str::<Argument>(r#"{"rules": [], "value": 42}"#).is_err());
    }
}
