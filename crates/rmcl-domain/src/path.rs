use std::path::Path;

/// Validates one portable path component without normalizing its identity.
pub fn validate_portable_component(component: &str) -> Result<(), &'static str> {
    if component.trim().is_empty() {
        return Err("path component is blank");
    }
    if component == "." || component == ".." {
        return Err("relative traversal components are not allowed");
    }
    if component.contains('/') {
        return Err("expected one path component");
    }
    if component.contains('\\') {
        return Err("backslashes are not portable path separators");
    }
    if component.contains(':') {
        return Err("colons and Windows drive prefixes are not allowed");
    }
    if component.chars().any(char::is_control) {
        return Err("control characters are not allowed");
    }
    if component.ends_with(' ') || component.ends_with('.') {
        return Err("path components cannot end in a dot or space");
    }

    let basename = component.split('.').next().unwrap_or(component);
    if is_windows_reserved_basename(basename) {
        return Err("Windows reserved device names are not allowed");
    }
    Ok(())
}

/// Validates a UTF-8 relative path using `/` separators on every platform.
pub fn validate_portable_path(path: &str) -> Result<(), &'static str> {
    if path.is_empty() {
        return Err("path is empty");
    }
    if path.starts_with('/') {
        return Err("absolute paths are not allowed");
    }
    for component in path.split('/') {
        if component.is_empty() {
            return Err("empty path components are not allowed");
        }
        validate_portable_component(component)?;
    }
    Ok(())
}

/// Produces a conservative case-insensitive key for detecting destination aliases.
pub fn portable_collision_key(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

/// Produces a collision key for a native destination without changing the destination itself.
pub fn destination_collision_key(path: impl AsRef<Path>) -> String {
    portable_collision_key(&path.as_ref().to_string_lossy())
}

fn is_windows_reserved_basename(basename: &str) -> bool {
    let uppercase = basename.to_ascii_uppercase();
    matches!(uppercase.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || uppercase.strip_prefix("COM").is_some_and(|suffix| {
            matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        })
        || uppercase.strip_prefix("LPT").is_some_and(|suffix| {
            matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_portable_unicode_paths() {
        assert_eq!(validate_portable_path("资源/界面 图标.png"), Ok(()));
        assert_eq!(validate_portable_path("com/example/lib-1.0.jar"), Ok(()));
    }

    #[test]
    fn rejects_windows_reserved_names_and_aliases() {
        for path in [
            "CON",
            "nul",
            "NUL.jar",
            "com1",
            "lpt9.txt",
            "safe/AUX/file",
            "name.",
            "name ",
        ] {
            assert!(validate_portable_path(path).is_err(), "accepted {path:?}");
        }
    }

    #[test]
    fn collision_keys_cover_case_and_separator_aliases() {
        assert_eq!(
            destination_collision_key("Native\\A.DLL"),
            destination_collision_key("native/a.dll")
        );
    }
}
