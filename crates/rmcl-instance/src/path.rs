use std::{fmt, path::Path};

use relative_path::RelativePathBuf;
use thiserror::Error;

/// A validated UTF-8 path relative to a Minecraft repository location.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RepositoryPath(RelativePathBuf);

impl RepositoryPath {
    pub fn new(path: impl Into<String>) -> Result<Self, LayoutError> {
        let path = path.into();
        validate_relative_path(&path).map_err(|reason| LayoutError::UnsafePath {
            path: path.clone(),
            reason,
        })?;
        Ok(Self(RelativePathBuf::from(path)))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn to_path(&self, base: impl AsRef<Path>) -> std::path::PathBuf {
        self.0.to_path(base)
    }
}

impl fmt::Display for RepositoryPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A Minecraft version identifier that is safe as one path component.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VersionId(String);

impl VersionId {
    pub fn new(id: impl Into<String>) -> Result<Self, LayoutError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(LayoutError::BlankVersionId);
        }
        validate_segment(&id).map_err(|_| LayoutError::InvalidVersionId(id.clone()))?;
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VersionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LayoutError {
    #[error("version id cannot be blank")]
    BlankVersionId,
    #[error("invalid version id `{0}`")]
    InvalidVersionId(String),
    #[error("unsafe repository path `{path}`: {reason}")]
    UnsafePath { path: String, reason: &'static str },
}

pub(crate) fn validate_segment(segment: &str) -> Result<(), &'static str> {
    if segment.trim().is_empty() {
        return Err("path component is blank");
    }
    if segment.contains('/') {
        return Err("expected one path component");
    }
    validate_relative_path(segment)
}

fn validate_relative_path(path: &str) -> Result<(), &'static str> {
    if path.is_empty() {
        return Err("path is empty");
    }
    if path.starts_with('/') {
        return Err("absolute paths are not allowed");
    }
    if path.ends_with('/') {
        return Err("path must identify a file or complete component");
    }
    if path.contains('\\') {
        return Err("backslashes are not portable repository separators");
    }
    if path.contains(':') {
        return Err("colons and Windows drive prefixes are not allowed");
    }
    if path.chars().any(char::is_control) {
        return Err("control characters are not allowed");
    }
    for segment in path.split('/') {
        match segment {
            "" => return Err("empty path components are not allowed"),
            "." | ".." => return Err("relative traversal components are not allowed"),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_portable_relative_paths() {
        let path = RepositoryPath::new("com/example/library/1.0/library-1.0.jar").unwrap();
        assert_eq!(path.as_str(), "com/example/library/1.0/library-1.0.jar");
        assert_eq!(
            path.to_path("root"),
            Path::new("root").join("com/example/library/1.0/library-1.0.jar")
        );
    }

    #[test]
    fn accepts_unicode_without_normalization() {
        let path = RepositoryPath::new("资源/界面 图标.png").unwrap();
        assert_eq!(path.as_str(), "资源/界面 图标.png");
    }

    #[test]
    fn rejects_unsafe_relative_paths() {
        for path in [
            "",
            "/absolute",
            "../escape",
            "safe/../escape",
            "safe//file",
            "safe/./file",
            "C:/escape",
            "safe\\escape",
            "trailing/",
            "nul\0byte",
        ] {
            assert!(RepositoryPath::new(path).is_err(), "accepted {path:?}");
        }
    }

    #[test]
    fn version_id_is_one_safe_component() {
        assert_eq!(VersionId::new("1.21.1").unwrap().as_str(), "1.21.1");
        for id in ["", " ", ".", "..", "a/b", "a\\b", "C:"] {
            assert!(VersionId::new(id).is_err(), "accepted {id:?}");
        }
    }
}
