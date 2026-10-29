use std::{
    collections::BTreeSet,
    io::Write,
    path::{Path, PathBuf},
};

use tempfile::NamedTempFile;

use crate::ContentError;

const RESOURCE_PACKS: &str = "resourcePacks";
const INCOMPATIBLE_RESOURCE_PACKS: &str = "incompatibleResourcePacks";

/// Direction to move an enabled resource pack in the priority list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourcePackOrder {
    /// Move toward the end of `resourcePacks`, where the pack overrides more.
    HigherPriority,
    /// Move toward the start of `resourcePacks`, where the pack overrides less.
    LowerPriority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResourcePackListFormat {
    Modern,
    Legacy,
}

impl ResourcePackListFormat {
    fn entry(self, file_name: &str) -> String {
        match self {
            Self::Modern => format!("file/{file_name}"),
            Self::Legacy => file_name.to_owned(),
        }
    }
}

/// A parsed `options.txt` view limited to the resource-pack lists.
#[derive(Debug, Clone)]
pub(crate) struct ResourcePackOptions {
    path: PathBuf,
    lines: Vec<String>,
    enabled: Vec<String>,
    incompatible: Vec<String>,
    format: ResourcePackListFormat,
}

impl ResourcePackOptions {
    pub(crate) fn load(path: PathBuf, modern_hint: bool) -> Result<Option<Self>, ContentError> {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(ContentError::Io {
                    operation: "read resource pack options",
                    path,
                    source,
                });
            }
        };
        let lines = text.lines().map(str::to_owned).collect::<Vec<_>>();
        let enabled = parse_list(&lines, RESOURCE_PACKS, &path)?;
        let incompatible = parse_list(&lines, INCOMPATIBLE_RESOURCE_PACKS, &path)?;
        let format = detect_format(&enabled, &incompatible, modern_hint);
        Ok(Some(Self {
            path,
            lines,
            enabled,
            incompatible,
            format,
        }))
    }

    pub(crate) fn new(path: PathBuf, modern_hint: bool) -> Self {
        Self {
            path,
            lines: Vec::new(),
            enabled: Vec::new(),
            incompatible: Vec::new(),
            format: if modern_hint {
                ResourcePackListFormat::Modern
            } else {
                ResourcePackListFormat::Legacy
            },
        }
    }

    pub(crate) fn enabled_names(&self) -> BTreeSet<String> {
        self.enabled.iter().map(|entry| normalize(entry)).collect()
    }

    pub(crate) fn enabled_file_names(&self) -> Vec<String> {
        self.enabled.iter().map(|entry| normalize(entry)).collect()
    }

    pub(crate) fn contains(&self, file_name: &str) -> bool {
        let target = normalize(file_name);
        self.enabled
            .iter()
            .chain(&self.incompatible)
            .any(|entry| normalize(entry) == target)
    }

    pub(crate) fn set_enabled(&mut self, file_name: &str, enabled: bool) {
        let target = normalize(file_name);
        self.enabled.retain(|entry| normalize(entry) != target);
        self.incompatible.retain(|entry| normalize(entry) != target);
        if enabled {
            self.enabled.push(self.format.entry(file_name));
        }
    }

    pub(crate) fn move_entry(
        &mut self,
        file_name: &str,
        order: ResourcePackOrder,
    ) -> Result<(), ContentError> {
        let target = normalize(file_name);
        let Some(index) = self
            .enabled
            .iter()
            .position(|entry| normalize(entry) == target)
        else {
            return Err(ContentError::ResourcePackNotEnabled {
                file_name: file_name.to_owned(),
            });
        };
        let neighbor = match order {
            ResourcePackOrder::HigherPriority => index.checked_add(1),
            ResourcePackOrder::LowerPriority => index.checked_sub(1),
        };
        if let Some(neighbor) = neighbor.filter(|neighbor| *neighbor < self.enabled.len()) {
            self.enabled.swap(index, neighbor);
        }
        Ok(())
    }

    pub(crate) fn save(&self) -> Result<(), ContentError> {
        let mut lines = self.lines.clone();
        set_line(&mut lines, RESOURCE_PACKS, &self.enabled, &self.path)?;
        set_line(
            &mut lines,
            INCOMPATIBLE_RESOURCE_PACKS,
            &self.incompatible,
            &self.path,
        )?;
        write_lines_atomic(&self.path, &lines)
    }
}

fn parse_list(lines: &[String], key: &str, path: &Path) -> Result<Vec<String>, ContentError> {
    let Some(line) = lines.iter().find(|line| line_key(line) == Some(key)) else {
        return Ok(Vec::new());
    };
    let value = line
        .split_once(':')
        .map(|(_, value)| value.trim())
        .unwrap_or_default();
    if value.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(value).map_err(|error| ContentError::Options {
        path: path.to_path_buf(),
        reason: format!("{key} is not a JSON string array: {error}"),
    })
}

fn set_line(
    lines: &mut Vec<String>,
    key: &str,
    entries: &[String],
    path: &Path,
) -> Result<(), ContentError> {
    let value = serde_json::to_string(entries).map_err(|error| ContentError::Options {
        path: path.to_path_buf(),
        reason: format!("failed to serialize {key}: {error}"),
    })?;
    let line = format!("{key}:{value}");
    match lines.iter().position(|line| line_key(line) == Some(key)) {
        Some(index) => lines[index] = line,
        None if !entries.is_empty() => lines.push(line),
        None => {}
    }
    Ok(())
}

fn line_key(line: &str) -> Option<&str> {
    line.split_once(':').map(|(key, _)| key.trim())
}

fn normalize(entry: &str) -> String {
    entry.strip_prefix("file/").unwrap_or(entry).to_owned()
}

fn detect_format(
    enabled: &[String],
    incompatible: &[String],
    modern_hint: bool,
) -> ResourcePackListFormat {
    if enabled
        .iter()
        .chain(incompatible)
        .any(|entry| entry.starts_with("file/"))
    {
        ResourcePackListFormat::Modern
    } else if !enabled.is_empty() || !incompatible.is_empty() {
        ResourcePackListFormat::Legacy
    } else if modern_hint {
        ResourcePackListFormat::Modern
    } else {
        ResourcePackListFormat::Legacy
    }
}

fn write_lines_atomic(path: &Path, lines: &[String]) -> Result<(), ContentError> {
    let parent = path.parent().ok_or_else(|| ContentError::Io {
        operation: "find resource pack options directory",
        path: path.to_path_buf(),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "options path has no parent",
        ),
    })?;
    std::fs::create_dir_all(parent).map_err(|source| ContentError::Io {
        operation: "create resource pack options directory",
        path: parent.to_path_buf(),
        source,
    })?;
    let mut text = lines.join("\n");
    text.push('\n');
    let mut temporary = NamedTempFile::new_in(parent).map_err(|source| ContentError::Io {
        operation: "create resource pack options temporary file",
        path: parent.to_path_buf(),
        source,
    })?;
    temporary
        .write_all(text.as_bytes())
        .map_err(|source| ContentError::Io {
            operation: "write resource pack options temporary file",
            path: temporary.path().to_path_buf(),
            source,
        })?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|source| ContentError::Io {
            operation: "sync resource pack options temporary file",
            path: temporary.path().to_path_buf(),
            source,
        })?;
    temporary.persist(path).map_err(|error| ContentError::Io {
        operation: "publish resource pack options",
        path: path.to_path_buf(),
        source: error.error,
    })?;
    Ok(())
}

/// Heuristic used when `options.txt` has no resource-pack entries yet.
///
/// Minecraft 1.13 switched from plain file names to `file/<name>` entries.
/// Unknown and snapshot-style versions predate neither convention, so modern
/// is the safer default; old alpha/beta and 1.12-and-earlier use the legacy
/// form.
pub(crate) fn modern_resource_pack_list(version: &str) -> bool {
    let version = version.trim();
    if let Some(rest) = version.strip_prefix("1.") {
        let minor = rest
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>();
        if let Ok(minor) = minor.parse::<u32>() {
            return minor >= 13;
        }
        return true;
    }
    if version.starts_with('a') || version.starts_with('b') {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parses_modern_and_legacy_lists() {
        let temporary = TempDir::new().unwrap();
        let modern = temporary.path().join("modern.txt");
        std::fs::write(&modern, "resourcePacks:[\"file/a.zip\",\"file/b.zip\"]\n").unwrap();
        let options = ResourcePackOptions::load(modern.clone(), false)
            .unwrap()
            .unwrap();
        assert!(options.enabled_names().contains("a.zip"));
        assert!(options.enabled_names().contains("b.zip"));

        let legacy = temporary.path().join("legacy.txt");
        std::fs::write(&legacy, "resourcePacks:[\"a.zip\"]\n").unwrap();
        let options = ResourcePackOptions::load(legacy, true).unwrap().unwrap();
        assert!(options.enabled_names().contains("a.zip"));
    }

    #[test]
    fn enables_at_the_end_and_disables_from_both_lists() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("options.txt");
        std::fs::write(
            &path,
            "language:zh_cn\nresourcePacks:[\"file/a.zip\"]\nincompatibleResourcePacks:[\"file/b.zip\"]\n",
        )
        .unwrap();
        let mut options = ResourcePackOptions::load(path.clone(), true)
            .unwrap()
            .unwrap();

        options.set_enabled("b.zip", true);
        options.save().unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("language:zh_cn"));
        assert!(saved.contains("resourcePacks:[\"file/a.zip\",\"file/b.zip\"]"));
        assert!(saved.contains("incompatibleResourcePacks:[]"));

        let mut options = ResourcePackOptions::load(path.clone(), true)
            .unwrap()
            .unwrap();
        options.set_enabled("a.zip", false);
        options.save().unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("resourcePacks:[\"file/b.zip\"]"));
    }

    #[test]
    fn moves_enabled_packs_in_priority_order() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("options.txt");
        std::fs::write(
            &path,
            "resourcePacks:[\"file/a.zip\",\"file/b.zip\",\"file/c.zip\"]\n",
        )
        .unwrap();
        let mut options = ResourcePackOptions::load(path.clone(), true)
            .unwrap()
            .unwrap();

        options
            .move_entry("a.zip", ResourcePackOrder::HigherPriority)
            .unwrap();
        assert_eq!(
            options.enabled,
            vec!["file/b.zip", "file/a.zip", "file/c.zip"]
        );
        options
            .move_entry("c.zip", ResourcePackOrder::LowerPriority)
            .unwrap();
        assert_eq!(
            options.enabled,
            vec!["file/b.zip", "file/c.zip", "file/a.zip"]
        );
        options
            .move_entry("a.zip", ResourcePackOrder::HigherPriority)
            .unwrap();
        assert_eq!(
            options.enabled,
            vec!["file/b.zip", "file/c.zip", "file/a.zip"]
        );
        options
            .move_entry("b.zip", ResourcePackOrder::LowerPriority)
            .unwrap();
        assert_eq!(
            options.enabled,
            vec!["file/b.zip", "file/c.zip", "file/a.zip"]
        );

        options.save().unwrap();
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "resourcePacks:[\"file/b.zip\",\"file/c.zip\",\"file/a.zip\"]\n"
        );
    }

    #[test]
    fn moving_a_disabled_pack_is_rejected() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("options.txt");
        std::fs::write(&path, "resourcePacks:[\"file/a.zip\"]\n").unwrap();
        let mut options = ResourcePackOptions::load(path, true).unwrap().unwrap();

        assert!(matches!(
            options.move_entry("missing.zip", ResourcePackOrder::HigherPriority),
            Err(ContentError::ResourcePackNotEnabled { .. })
        ));
    }

    #[test]
    fn malformed_options_are_rejected_without_rewriting() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("options.txt");
        std::fs::write(&path, "resourcePacks:not-json\n").unwrap();

        assert!(matches!(
            ResourcePackOptions::load(path.clone(), true),
            Err(ContentError::Options { .. })
        ));
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "resourcePacks:not-json\n"
        );
    }

    #[test]
    fn version_heuristic_handles_modern_legacy_and_snapshots() {
        assert!(modern_resource_pack_list("1.21.1"));
        assert!(modern_resource_pack_list("1.13"));
        assert!(!modern_resource_pack_list("1.12.2"));
        assert!(modern_resource_pack_list("24w14a"));
        assert!(!modern_resource_pack_list("b1.7.3"));
    }
}
