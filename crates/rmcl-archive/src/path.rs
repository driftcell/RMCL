use std::path::{Path, PathBuf};

use relative_path::RelativePathBuf;
use rmcl_domain::{portable_collision_key, validate_portable_path};

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
            collision_key: portable_collision_key(relative),
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
    let name = name.strip_suffix('/').unwrap_or(name);
    validate_portable_path(name)?;
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
            "NUL",
            "safe/LPT1.jar",
            "trailing.",
            "trailing ",
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
