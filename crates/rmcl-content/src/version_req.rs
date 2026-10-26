//! Version requirement matching for mod dependency metadata.
//!
//! Fabric/Quilt versions are semver-like and are evaluated with the `semver`
//! crate after translating Fabric's predicate syntax. Forge/NeoForge ranges use
//! Maven's `[1.20,1.21)` syntax; the small range parser here delegates version
//! ordering to `mvn_version`'s Maven `ComparableVersion`.

use mvn_version::ComparableVersion;
use semver::{Version, VersionReq};
use thiserror::Error;

/// Format used by a mod loader's dependency declarations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionStyle {
    /// Fabric/Quilt predicates: `>=1.0.0 <2.0.0`, `1.2.x`, `~1.2.3`, `^1.2.3`.
    Semver,
    /// Forge/NeoForge Maven ranges: `[1.20,1.21)`, `[1.0]`, `(,1.20]`.
    Maven,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRequirement {
    kind: RequirementKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RequirementKind {
    Any,
    Semver(Vec<VersionReq>),
    Maven(Vec<MavenInterval>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MavenInterval {
    lower: Option<String>,
    lower_inclusive: bool,
    upper: Option<String>,
    upper_inclusive: bool,
}

impl VersionRequirement {
    pub fn parse(style: VersionStyle, requirement: &str) -> Result<Self, VersionRequirementError> {
        let requirement = requirement.trim();
        if requirement.is_empty() || requirement == "*" {
            return Ok(Self {
                kind: RequirementKind::Any,
            });
        }
        let kind = match style {
            VersionStyle::Semver => {
                RequirementKind::Semver(parse_semver_requirements(requirement)?)
            }
            VersionStyle::Maven => RequirementKind::Maven(parse_maven_ranges(requirement)?),
        };
        Ok(Self { kind })
    }

    pub fn matches(&self, version: &str) -> bool {
        match &self.kind {
            RequirementKind::Any => true,
            RequirementKind::Semver(requirements) => {
                let Some(version) = parse_semver_version(version) else {
                    return false;
                };
                requirements
                    .iter()
                    .any(|requirement| requirement.matches(&version))
            }
            RequirementKind::Maven(intervals) => {
                let version = ComparableVersion::new(version.trim());
                intervals.iter().any(|interval| interval.matches(&version))
            }
        }
    }
}

/// Returns `None` when the requirement or installed version cannot be parsed.
pub fn version_satisfies(version: &str, requirement: &str, style: VersionStyle) -> Option<bool> {
    let requirement = VersionRequirement::parse(style, requirement).ok()?;
    match style {
        VersionStyle::Semver => parse_semver_version(version).map(|_| requirement.matches(version)),
        VersionStyle::Maven => Some(requirement.matches(version)),
    }
}

fn parse_semver_version(version: &str) -> Option<Version> {
    let version = version.trim();
    Version::parse(version).ok().or_else(|| {
        normalize_semver(version)
            .ok()
            .and_then(|normalized| Version::parse(&normalized).ok())
    })
}

impl MavenInterval {
    fn all() -> Self {
        Self {
            lower: None,
            lower_inclusive: true,
            upper: None,
            upper_inclusive: true,
        }
    }

    fn exact(version: &str) -> Self {
        Self {
            lower: Some(version.to_owned()),
            lower_inclusive: true,
            upper: Some(version.to_owned()),
            upper_inclusive: true,
        }
    }

    fn matches(&self, version: &ComparableVersion) -> bool {
        if let Some(lower) = &self.lower {
            let lower = ComparableVersion::new(lower);
            let ordering = version.cmp(&lower);
            if self.lower_inclusive {
                if ordering.is_lt() {
                    return false;
                }
            } else if !ordering.is_gt() {
                return false;
            }
        }
        if let Some(upper) = &self.upper {
            let upper = ComparableVersion::new(upper);
            let ordering = version.cmp(&upper);
            if self.upper_inclusive {
                if ordering.is_gt() {
                    return false;
                }
            } else if !ordering.is_lt() {
                return false;
            }
        }
        true
    }
}

fn parse_semver_requirements(
    requirement: &str,
) -> Result<Vec<VersionReq>, VersionRequirementError> {
    let mut requirements = Vec::new();
    for alternative in requirement.split("||") {
        let alternative = alternative.trim();
        if alternative.is_empty() {
            return Err(VersionRequirementError::Invalid(requirement.to_owned()));
        }
        let mut predicates = Vec::new();
        for predicate in alternative.split_whitespace() {
            predicates.push(convert_semver_predicate(predicate)?);
        }
        let combined = predicates.join(", ");
        let requirement = VersionReq::parse(&combined)
            .map_err(|_| VersionRequirementError::Invalid(requirement.to_owned()))?;
        requirements.push(requirement);
    }
    Ok(requirements)
}

fn convert_semver_predicate(predicate: &str) -> Result<String, VersionRequirementError> {
    let (operator, value) = if let Some(value) = predicate.strip_prefix(">=") {
        (">=", value)
    } else if let Some(value) = predicate.strip_prefix("<=") {
        ("<=", value)
    } else if let Some(value) = predicate.strip_prefix('>') {
        (">", value)
    } else if let Some(value) = predicate.strip_prefix('<') {
        ("<", value)
    } else if let Some(value) = predicate.strip_prefix('=') {
        ("=", value)
    } else if let Some(value) = predicate.strip_prefix('~') {
        ("~", value)
    } else if let Some(value) = predicate.strip_prefix('^') {
        ("^", value)
    } else {
        ("", predicate)
    };
    if value.is_empty() {
        return Err(VersionRequirementError::Invalid(predicate.to_owned()));
    }
    if value == "*" {
        return Ok("*".to_owned());
    }
    if value.contains(['x', 'X', '*']) {
        if !operator.is_empty() {
            return Err(VersionRequirementError::Invalid(predicate.to_owned()));
        }
        return wildcard_predicate(value);
    }
    let version = normalize_semver(value)?;
    if operator.is_empty() {
        Ok(format!("={version}"))
    } else {
        Ok(format!("{operator}{version}"))
    }
}

fn wildcard_predicate(value: &str) -> Result<String, VersionRequirementError> {
    let parts = value.split('.').collect::<Vec<_>>();
    let wildcard = parts
        .iter()
        .position(|part| matches!(*part, "x" | "X" | "*"));
    let Some(wildcard) = wildcard else {
        return Err(VersionRequirementError::Invalid(value.to_owned()));
    };
    if parts[wildcard + 1..]
        .iter()
        .any(|part| !matches!(*part, "x" | "X" | "*"))
    {
        return Err(VersionRequirementError::Invalid(value.to_owned()));
    }
    let normalized = parts[..wildcard]
        .iter()
        .map(|part| {
            part.parse::<u64>()
                .map(|number| number.to_string())
                .map_err(|_| VersionRequirementError::Invalid(value.to_owned()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(if normalized.is_empty() {
        "*".to_owned()
    } else {
        format!("{}.*", normalized.join("."))
    })
}

fn normalize_semver(value: &str) -> Result<String, VersionRequirementError> {
    let value = value.strip_prefix('v').unwrap_or(value);
    let value = value.split('+').next().unwrap_or(value);
    let (core, prerelease) = match value.split_once('-') {
        Some((core, prerelease)) => (core, Some(prerelease)),
        None => (value, None),
    };
    let parts = core.split('.').collect::<Vec<_>>();
    if parts.is_empty() || parts.len() > 3 {
        return Err(VersionRequirementError::Invalid(value.to_owned()));
    }
    let mut numbers = parts
        .iter()
        .map(|part| {
            part.parse::<u64>()
                .map(|number| number.to_string())
                .map_err(|_| VersionRequirementError::Invalid(value.to_owned()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    while numbers.len() < 3 {
        numbers.push("0".to_owned());
    }
    let mut normalized = numbers.join(".");
    if let Some(prerelease) = prerelease {
        normalized.push('-');
        normalized.push_str(prerelease);
    }
    Ok(normalized)
}

fn parse_maven_ranges(requirement: &str) -> Result<Vec<MavenInterval>, VersionRequirementError> {
    if !requirement.starts_with(['[', '(']) {
        return Ok(vec![MavenInterval::exact(requirement)]);
    }

    let mut intervals = Vec::new();
    let mut index = 0;
    while index < requirement.len() {
        let start = requirement[index..]
            .chars()
            .next()
            .ok_or_else(|| VersionRequirementError::Invalid(requirement.to_owned()))?;
        if start != '[' && start != '(' {
            return Err(VersionRequirementError::Invalid(requirement.to_owned()));
        }
        let close_offset = requirement[index..]
            .char_indices()
            .find(|(_, character)| matches!(character, ']' | ')'))
            .map(|(offset, _)| offset)
            .ok_or_else(|| VersionRequirementError::Invalid(requirement.to_owned()))?;
        let end = requirement[index + close_offset..]
            .chars()
            .next()
            .ok_or_else(|| VersionRequirementError::Invalid(requirement.to_owned()))?;
        let body = &requirement[index + 1..index + close_offset];
        let bounds = body.split(',').map(str::trim).collect::<Vec<_>>();
        let interval = match bounds.as_slice() {
            [version] if start == '[' && end == ']' && !version.is_empty() => {
                MavenInterval::exact(version)
            }
            [lower, upper] => {
                let lower = (!lower.is_empty()).then(|| (*lower).to_owned());
                let upper = (!upper.is_empty()).then(|| (*upper).to_owned());
                if lower.is_none() && upper.is_none() {
                    MavenInterval::all()
                } else {
                    let interval = MavenInterval {
                        lower,
                        lower_inclusive: start == '[',
                        upper,
                        upper_inclusive: end == ']',
                    };
                    if let (Some(lower), Some(upper)) =
                        (interval.lower.as_ref(), interval.upper.as_ref())
                    {
                        let lower_version = ComparableVersion::new(lower);
                        let upper_version = ComparableVersion::new(upper);
                        if lower_version > upper_version {
                            return Err(VersionRequirementError::Invalid(requirement.to_owned()));
                        }
                    }
                    interval
                }
            }
            _ => return Err(VersionRequirementError::Invalid(requirement.to_owned())),
        };
        intervals.push(interval);
        index += close_offset + 1;
        if index == requirement.len() {
            break;
        }
        if requirement[index..].starts_with(',') {
            index += 1;
            while requirement[index..].starts_with(' ') {
                index += 1;
            }
        } else {
            return Err(VersionRequirementError::Invalid(requirement.to_owned()));
        }
    }
    Ok(intervals)
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VersionRequirementError {
    #[error("invalid version requirement `{0}`")]
    Invalid(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn semver(version: &str, requirement: &str) -> bool {
        version_satisfies(version, requirement, VersionStyle::Semver).unwrap_or(false)
    }

    fn maven(version: &str, requirement: &str) -> bool {
        version_satisfies(version, requirement, VersionStyle::Maven).unwrap_or(false)
    }

    #[test]
    fn matches_fabric_exact_and_comparators() {
        assert!(semver("1.2.3", "1.2.3"));
        assert!(!semver("1.2.4", "1.2.3"));
        assert!(semver("0.100.0", ">=0.92.0"));
        assert!(!semver("0.91.9", ">=0.92.0"));
        assert!(semver("1.20.4", ">=1.20 <1.21"));
        assert!(!semver("1.21.0", ">=1.20 <1.21"));
    }

    #[test]
    fn matches_fabric_wildcards_tilde_and_caret() {
        assert!(semver("1.20.4", "1.20.x"));
        assert!(!semver("1.21.0", "1.20.x"));
        assert!(semver("1.2.9", "~1.2.3"));
        assert!(!semver("1.3.0", "~1.2.3"));
        assert!(semver("0.2.9", "^0.2.3"));
        assert!(!semver("0.3.0", "^0.2.3"));
        assert!(semver("2.1.0", "1.0.0 || >=2.0.0"));
        assert!(!semver("1.5.0", "1.0.0 || >=2.0.0"));
        assert!(semver("2.0", ">=0.5.0"));
    }

    #[test]
    fn matches_maven_ranges() {
        assert!(maven("1.0", "[1.0,2.0)"));
        assert!(maven("1.9.9", "[1.0,2.0)"));
        assert!(!maven("2.0", "[1.0,2.0)"));
        assert!(maven("0.9", "(,1.0]"));
        assert!(maven("1.0", "(,1.0]"));
        assert!(!maven("1.1", "(,1.0]"));
        assert!(maven("1.0", "[1.0]"));
        assert!(!maven("1.0.1", "[1.0]"));
        assert!(maven("1.20.1-47.2.0", "[1.20,1.21)"));
        assert!(!maven("1.21", "[1.20,1.21)"));
        assert!(maven("0.9", "(,1.0],[1.2,)"));
        assert!(maven("1.3", "(,1.0],[1.2,)"));
        assert!(!maven("1.1", "(,1.0],[1.2,)"));
    }

    #[test]
    fn invalid_requirements_return_unknown() {
        assert_eq!(
            version_satisfies("1.0.0", "not a version", VersionStyle::Semver),
            None
        );
        assert_eq!(
            version_satisfies("1.0", "[2.0,1.0]", VersionStyle::Maven),
            None
        );
        assert_eq!(
            version_satisfies("not-semver", ">=1.0.0", VersionStyle::Semver),
            None
        );
    }
}
