use std::path::{Path, PathBuf};

use relative_path::RelativePathBuf;

use crate::ExtractionError;

#[derive(Debug, Clone)]
pub(crate) struct EntryPath {
    relative: RelativePathBuf,
    collision_key: String,
}

impl EntryPath {
    pub(crate) fn new(name: &str) -> Result<Self, ExtractionError> {
        let relative = validate(name).map_err(|reason| ExtractionError::UnsafeEntry {
            entry: name.to_owned(),
            reason,
        })?;
        Ok(Self {
            relative: RelativePathBuf::from(relative),
            collision_key: relative.to_lowercase(),
        })
    }

    pub(crate) fn collision_key(&self) -> &str {
        &self.collision_key
    }

    pub(crate) fn to_path(&self, base: impl AsRef<Path>) -> PathBuf {
        self.relative.to_path(base)
    }
}

fn validate(name: &str) -> Result<&str, &'static str> {
    if name.is_empty() {
        return Err("entry name is empty");
    }
    if name.starts_with('/') {
        return Err("absolute paths are not allowed");
    }
    if name.contains('\\') {
        return Err("backslashes are not allowed");
    }
    if name.contains(':') {
        return Err("drive prefixes and alternate data streams are not allowed");
    }
    if name.chars().any(char::is_control) {
        return Err("control characters are not allowed");
    }

    let name = name.strip_suffix('/').unwrap_or(name);
    if name.is_empty() {
        return Err("entry has no relative path");
    }
    for component in name.split('/') {
        match component {
            "" => return Err("empty path components are not allowed"),
            "." | ".." => return Err("traversal components are not allowed"),
            _ => {}
        }
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_entry_paths() {
        let path = EntryPath::new("native/lib/example.so").unwrap();
        assert_eq!(path.collision_key(), "native/lib/example.so");
        assert_eq!(
            path.to_path("root"),
            Path::new("root").join("native/lib/example.so")
        );
    }

    #[test]
    fn rejects_unsafe_entry_paths() {
        for name in [
            "",
            "/absolute",
            "../escape",
            "safe/../escape",
            "safe//file",
            "safe/./file",
            "C:/escape",
            "safe\\escape",
            "nul\0byte",
            "/",
        ] {
            assert!(EntryPath::new(name).is_err(), "accepted {name:?}");
        }
    }

    #[test]
    fn collision_key_is_case_insensitive() {
        assert_eq!(
            EntryPath::new("Native/A.DLL").unwrap().collision_key(),
            EntryPath::new("native/a.dll").unwrap().collision_key()
        );
    }
}
