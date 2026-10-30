use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

use flate2::read::GzDecoder;
use serde::Deserialize;

use crate::WorldError;

const MAX_LEVEL_DAT_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Default)]
pub(crate) struct LevelMetadata {
    pub name: Option<String>,
    pub game_version: Option<String>,
    pub data_version: Option<i32>,
    pub last_played: Option<i64>,
    pub was_modded: bool,
    pub enabled_features: Vec<String>,
    pub enabled_datapacks: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LevelRoot {
    data: LevelData,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LevelData {
    level_name: Option<String>,
    data_version: Option<i32>,
    version: Option<LevelVersion>,
    last_played: Option<i64>,
    was_modded: Option<i8>,
    data_packs: Option<DataPacks>,
    enabled_features: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LevelVersion {
    id: Option<i32>,
    name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DataPacks {
    enabled: Option<Vec<String>>,
}

pub(crate) fn read_level_dat(path: &Path) -> Result<LevelMetadata, WorldError> {
    let file = File::open(path).map_err(|source| WorldError::io(path, source))?;
    let reader = BufReader::new(file).take(MAX_LEVEL_DAT_BYTES);
    let decoder = GzDecoder::new(reader);
    let limited = decoder.take(MAX_LEVEL_DAT_BYTES);
    let root: LevelRoot = fastnbt::from_reader(limited).map_err(|source| WorldError::Nbt {
        path: path.to_path_buf(),
        source,
    })?;
    let data = root.data;
    let name = data.level_name.map(|name| strip_section_codes(&name));
    let game_version = data
        .version
        .as_ref()
        .and_then(|version| version.name.clone());
    let data_version = data
        .data_version
        .or_else(|| data.version.as_ref().and_then(|version| version.id));
    Ok(LevelMetadata {
        name,
        game_version,
        data_version,
        last_played: data.last_played,
        was_modded: data.was_modded.unwrap_or_default() != 0,
        enabled_features: data.enabled_features.unwrap_or_default(),
        enabled_datapacks: data
            .data_packs
            .and_then(|packs| packs.enabled)
            .unwrap_or_default(),
    })
}

pub(crate) fn strip_section_codes(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character == '§' {
            let _ = chars.next();
        } else {
            output.push(character);
        }
    }
    output.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_section_codes_without_touching_unicode() {
        assert_eq!(strip_section_codes("§a生存 §l世界"), "生存 世界");
        assert_eq!(strip_section_codes("普通世界"), "普通世界");
    }
}
