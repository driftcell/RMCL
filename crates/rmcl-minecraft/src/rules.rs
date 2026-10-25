use std::collections::BTreeMap;

use regex::Regex;
use serde::Deserialize;
use thiserror::Error;

pub type FeatureMap = BTreeMap<String, bool>;

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub os: OperatingSystem,
    pub os_version: String,
    pub arch: String,
}

impl Platform {
    pub fn new(
        os: OperatingSystem,
        os_version: impl Into<String>,
        arch: impl Into<String>,
    ) -> Self {
        Self {
            os,
            os_version: os_version.into(),
            arch: arch.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    #[default]
    Allow,
    Disallow,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct OsRestriction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Rule {
    #[serde(default)]
    pub action: RuleAction,
    #[serde(default)]
    pub os: Option<OsRestriction>,
    #[serde(default)]
    pub features: FeatureMap,
}

/// Evaluates an ordered Minecraft rule list against one launch environment.
///
/// Empty lists allow the item. Otherwise evaluation starts denied and the last matching rule wins.
pub fn evaluate_rules(
    rules: &[Rule],
    platform: &Platform,
    features: &FeatureMap,
) -> Result<bool, RuleError> {
    if rules.is_empty() {
        return Ok(true);
    }

    let mut allowed = false;
    for rule in rules {
        if rule_applies(rule, platform, features)? {
            allowed = rule.action == RuleAction::Allow;
        }
    }
    Ok(allowed)
}

#[derive(Debug, Error)]
pub enum RuleError {
    #[error("invalid regular expression for OS {field}: `{pattern}`")]
    InvalidOsPattern {
        field: &'static str,
        pattern: String,
        #[source]
        source: regex::Error,
    },
}

fn rule_applies(
    rule: &Rule,
    platform: &Platform,
    features: &FeatureMap,
) -> Result<bool, RuleError> {
    if let Some(restriction) = &rule.os
        && !os_matches(restriction, platform)?
    {
        return Ok(false);
    }

    // NOTE: Standard launch features are boolean switches; an omitted switch is disabled.
    Ok(rule
        .features
        .iter()
        .all(|(name, expected)| features.get(name).copied().unwrap_or(false) == *expected))
}

fn os_matches(restriction: &OsRestriction, platform: &Platform) -> Result<bool, RuleError> {
    if let Some(name) = &restriction.name {
        let Some(required_os) = OperatingSystem::from_manifest_name(name) else {
            return Ok(false);
        };
        if required_os != platform.os {
            return Ok(false);
        }
    }

    if let Some(pattern) = &restriction.version
        && !matches_entire(pattern, &platform.os_version, "version")?
    {
        return Ok(false);
    }

    if let Some(pattern) = &restriction.arch
        && !matches_entire(pattern, &platform.arch, "arch")?
    {
        return Ok(false);
    }

    Ok(true)
}

fn matches_entire(pattern: &str, value: &str, field: &'static str) -> Result<bool, RuleError> {
    let anchored_pattern = format!("^(?:{pattern})$");
    let regex = Regex::new(&anchored_pattern).map_err(|source| RuleError::InvalidOsPattern {
        field,
        pattern: pattern.to_owned(),
        source,
    })?;
    Ok(regex.is_match(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows() -> Platform {
        Platform::new(OperatingSystem::Windows, "10.0.22631", "x86_64")
    }

    fn os_rule(action: RuleAction, name: &str) -> Rule {
        Rule {
            action,
            os: Some(OsRestriction {
                name: Some(name.to_owned()),
                ..OsRestriction::default()
            }),
            features: FeatureMap::new(),
        }
    }

    #[test]
    fn empty_rules_allow() {
        assert!(evaluate_rules(&[], &windows(), &FeatureMap::new()).unwrap());
    }

    #[test]
    fn nonempty_rules_start_denied() {
        let rules = [os_rule(RuleAction::Allow, "linux")];
        assert!(!evaluate_rules(&rules, &windows(), &FeatureMap::new()).unwrap());
    }

    #[test]
    fn last_matching_rule_wins() {
        let rules = [
            os_rule(RuleAction::Allow, "windows"),
            os_rule(RuleAction::Disallow, "windows"),
        ];
        assert!(!evaluate_rules(&rules, &windows(), &FeatureMap::new()).unwrap());

        let rules = [
            os_rule(RuleAction::Disallow, "windows"),
            os_rule(RuleAction::Allow, "windows"),
        ];
        assert!(evaluate_rules(&rules, &windows(), &FeatureMap::new()).unwrap());
    }

    #[test]
    fn omitted_action_defaults_to_allow() {
        let rule: Rule = serde_json::from_str("{}").unwrap();
        assert!(evaluate_rules(&[rule], &windows(), &FeatureMap::new()).unwrap());
    }

    #[test]
    fn matches_each_supported_os_name() {
        for (name, os) in [
            ("windows", OperatingSystem::Windows),
            ("osx", OperatingSystem::MacOs),
            ("linux", OperatingSystem::Linux),
        ] {
            let platform = Platform::new(os, "1", "x86_64");
            let rules = [os_rule(RuleAction::Allow, name)];
            assert!(evaluate_rules(&rules, &platform, &FeatureMap::new()).unwrap());
            assert_eq!(OperatingSystem::from_manifest_name(name), Some(os));
            assert_eq!(os.as_str(), name);
        }
    }

    #[test]
    fn unknown_os_restriction_does_not_match() {
        let rules = [os_rule(RuleAction::Allow, "plan9")];
        assert!(!evaluate_rules(&rules, &windows(), &FeatureMap::new()).unwrap());
    }

    #[test]
    fn version_and_arch_patterns_cover_the_entire_value() {
        let rules = [Rule {
            action: RuleAction::Allow,
            os: Some(OsRestriction {
                name: None,
                version: Some(r"^10\..*".to_owned()),
                arch: Some("x86|x86_64".to_owned()),
            }),
            features: FeatureMap::new(),
        }];
        assert!(evaluate_rules(&rules, &windows(), &FeatureMap::new()).unwrap());
        assert!(!matches_entire("x86", "x86_64", "arch").unwrap());
        assert!(!matches_entire("10", "10.0", "version").unwrap());
    }

    #[test]
    fn invalid_patterns_are_reported() {
        let error = matches_entire("[", "anything", "version").unwrap_err();
        assert!(matches!(
            error,
            RuleError::InvalidOsPattern {
                field: "version",
                ..
            }
        ));
    }

    #[test]
    fn missing_features_are_false() {
        let mut expected = FeatureMap::new();
        expected.insert("is_demo_user".to_owned(), false);
        let rules = [Rule {
            action: RuleAction::Allow,
            os: None,
            features: expected,
        }];
        assert!(evaluate_rules(&rules, &windows(), &FeatureMap::new()).unwrap());
    }

    #[test]
    fn all_features_must_match() {
        let mut expected = FeatureMap::new();
        expected.insert("is_demo_user".to_owned(), true);
        expected.insert("has_custom_resolution".to_owned(), true);
        let rules = [Rule {
            action: RuleAction::Allow,
            os: None,
            features: expected,
        }];

        let mut actual = FeatureMap::new();
        actual.insert("is_demo_user".to_owned(), true);
        assert!(!evaluate_rules(&rules, &windows(), &actual).unwrap());
        actual.insert("has_custom_resolution".to_owned(), true);
        assert!(evaluate_rules(&rules, &windows(), &actual).unwrap());
    }
}
