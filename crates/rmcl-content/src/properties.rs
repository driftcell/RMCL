//! Small Java-properties editor for Minecraft-owned shader configuration.
//!
//! Minecraft stores shader selections in `iris.properties`,
//! `oculus.properties`, and `optionsshaders.txt`. The files are Java
//! properties: non-ASCII characters are written as `\uXXXX` escapes and
//! unknown keys/comments must survive an edit. This type only replaces known
//! keys line by line; it does not rewrite the whole file from a map.

use std::{fs, io::Write, path::PathBuf};

use tempfile::NamedTempFile;

use crate::ContentError;

pub(crate) struct PropertiesFile {
    path: PathBuf,
    lines: Vec<String>,
}

impl PropertiesFile {
    pub(crate) fn load(path: PathBuf) -> Result<Option<Self>, ContentError> {
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(ContentError::Io {
                    operation: "read shader configuration",
                    path,
                    source,
                });
            }
        };
        Ok(Some(Self {
            path,
            lines: text.lines().map(str::to_owned).collect(),
        }))
    }

    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            lines: Vec::new(),
        }
    }

    pub(crate) fn get(&self, key: &str) -> Option<String> {
        self.lines
            .iter()
            .rev()
            .find_map(|line| property_entry(line).filter(|(name, _)| name == key))
            .map(|(_, value)| value)
    }

    pub(crate) fn set(&mut self, key: &str, value: &str) {
        let line = format!("{key}={}", encode_value(value));
        if let Some(index) = self
            .lines
            .iter()
            .rposition(|line| property_entry(line).is_some_and(|(name, _)| name == key))
        {
            self.lines[index] = line;
        } else {
            self.lines.push(line);
        }
    }

    pub(crate) fn save(&self) -> Result<(), ContentError> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| ContentError::ShaderConfig {
                path: self.path.clone(),
                reason: "configuration path has no parent directory".to_owned(),
            })?;
        fs::create_dir_all(parent).map_err(|source| ContentError::Io {
            operation: "create shader configuration directory",
            path: parent.to_path_buf(),
            source,
        })?;
        let mut text = self.lines.join("\n");
        if !text.is_empty() {
            text.push('\n');
        }
        let mut temporary = NamedTempFile::new_in(parent).map_err(|source| ContentError::Io {
            operation: "create shader configuration temporary file",
            path: parent.to_path_buf(),
            source,
        })?;
        temporary
            .write_all(text.as_bytes())
            .map_err(|source| ContentError::Io {
                operation: "write shader configuration temporary file",
                path: temporary.path().to_path_buf(),
                source,
            })?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|source| ContentError::Io {
                operation: "sync shader configuration temporary file",
                path: temporary.path().to_path_buf(),
                source,
            })?;
        temporary
            .persist(&self.path)
            .map_err(|error| ContentError::Io {
                operation: "publish shader configuration",
                path: self.path.clone(),
                source: error.error,
            })?;
        Ok(())
    }
}

fn property_entry(line: &str) -> Option<(String, String)> {
    let line = line.trim_start();
    if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
        return None;
    }
    let (key, value) = split_entry(line)?;
    Some((decode_key(key)?, decode_value(value)?))
}

fn split_entry(line: &str) -> Option<(&str, &str)> {
    let mut escaped = false;
    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' => escaped = true,
            '=' | ':' => return Some((&line[..index], &line[index + 1..])),
            character if character.is_whitespace() => {
                let rest = &line[index + character.len_utf8()..];
                let rest = rest.trim_start_matches(char::is_whitespace);
                let rest = rest
                    .strip_prefix('=')
                    .or_else(|| rest.strip_prefix(':'))
                    .unwrap_or(rest);
                return Some((&line[..index], rest));
            }
            _ => {}
        }
    }
    Some((line, ""))
}

fn decode_key(value: &str) -> Option<String> {
    decode_escapes(value)
}

fn decode_value(value: &str) -> Option<String> {
    decode_escapes(value)
}

fn decode_escapes(value: &str) -> Option<String> {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '\\' {
            result.push(character);
            continue;
        }
        let Some(escaped) = chars.next() else {
            break;
        };
        match escaped {
            't' => result.push('\t'),
            'n' => result.push('\n'),
            'r' => result.push('\r'),
            'f' => result.push('\u{000c}'),
            'u' => {
                let unit = read_hex_escape(&mut chars)?;
                if (0xd800..=0xdbff).contains(&unit) {
                    let saved = chars.clone();
                    let mut pair = chars.clone();
                    if pair.next() == Some('\\')
                        && pair.next() == Some('u')
                        && let Some(low) = read_hex_escape(&mut pair)
                        && (0xdc00..=0xdfff).contains(&low)
                    {
                        let scalar = 0x1_0000
                            + ((u32::from(unit) - 0xd800) << 10)
                            + (u32::from(low) - 0xdc00);
                        result.push(char::from_u32(scalar)?);
                        chars = pair;
                        continue;
                    }
                    chars = saved;
                    result.push(char::REPLACEMENT_CHARACTER);
                } else if (0xdc00..=0xdfff).contains(&unit) {
                    result.push(char::REPLACEMENT_CHARACTER);
                } else {
                    result.push(char::from_u32(u32::from(unit))?);
                }
            }
            other => result.push(other),
        }
    }
    Some(result)
}

fn read_hex_escape(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<u16> {
    let mut value = 0u16;
    for _ in 0..4 {
        value = value.checked_mul(16)? + chars.next()?.to_digit(16)? as u16;
    }
    Some(value)
}

fn encode_value(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    let mut first = true;
    for character in value.chars() {
        match character {
            '\\' => encoded.push_str("\\\\"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            '=' => encoded.push_str("\\="),
            ':' => encoded.push_str("\\:"),
            '#' => encoded.push_str("\\#"),
            '!' => encoded.push_str("\\!"),
            ' ' if first => encoded.push_str("\\ "),
            character if !character.is_ascii() => {
                let mut units = [0u16; 2];
                for unit in character.encode_utf16(&mut units) {
                    encoded.push_str(&format!("\\u{unit:04x}"));
                }
            }
            character => encoded.push(character),
        }
        first = false;
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use tempfile::TempDir;

    #[test]
    fn edits_known_keys_without_touching_comments_or_unknown_keys() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("iris.properties");
        fs::write(
            &path,
            "# keep\ncolorSpace=SRGB\nshaderPack=old.zip\nenableShaders=false\n",
        )
        .unwrap();
        let mut file = PropertiesFile::load(path.clone()).unwrap().unwrap();
        assert_eq!(file.get("shaderPack").as_deref(), Some("old.zip"));
        file.set("shaderPack", "new pack.zip");
        file.set("enableShaders", "true");
        file.save().unwrap();

        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("# keep"));
        assert!(saved.contains("colorSpace=SRGB"));
        assert!(saved.contains("shaderPack=new pack.zip"));
        assert!(saved.contains("enableShaders=true"));

        let loaded = PropertiesFile::load(path).unwrap().unwrap();
        assert_eq!(loaded.get("shaderPack").as_deref(), Some("new pack.zip"));
    }

    #[test]
    fn decodes_minecraft_unicode_escapes() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("optionsshaders.txt");
        fs::write(&path, "shaderPack=\\u5149\\u5f71.zip\n").unwrap();
        let file = PropertiesFile::load(path).unwrap().unwrap();
        assert_eq!(file.get("shaderPack").as_deref(), Some("光影.zip"));
    }

    #[test]
    fn encodes_non_ascii_and_preserves_round_trip() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("optionsshaders.txt");
        let mut file = PropertiesFile::new(path.clone());
        file.set("shaderPack", "光影.zip");
        file.save().unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "shaderPack=\\u5149\\u5f71.zip\n"
        );
        let loaded = PropertiesFile::load(path).unwrap().unwrap();
        assert_eq!(loaded.get("shaderPack").as_deref(), Some("光影.zip"));
    }

    #[test]
    fn creates_missing_parent_directories() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("config/iris.properties");
        let mut file = PropertiesFile::new(path.clone());
        file.set("shaderPack", "pack.zip");
        file.save().unwrap();
        assert!(path.is_file());
    }

    #[test]
    fn escapes_separator_characters_in_values() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("shader.properties");
        let mut file = PropertiesFile::new(path.clone());
        file.set("shaderPack", "a=b:c#d!e\\f.zip");
        file.save().unwrap();
        let loaded = PropertiesFile::load(path).unwrap().unwrap();
        assert_eq!(
            loaded.get("shaderPack").as_deref(),
            Some("a=b:c#d!e\\f.zip")
        );
    }

    #[test]
    fn ignores_comments_and_lines_without_separators() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("shader.properties");
        fs::write(&path, "# comment\n! comment\nshaderPack=pack.zip\n").unwrap();
        let file = PropertiesFile::load(path).unwrap().unwrap();
        assert_eq!(file.get("shaderPack").as_deref(), Some("pack.zip"));
        assert_eq!(file.get("missing"), None);
        assert!(!Path::new("# comment").exists());
    }
}
