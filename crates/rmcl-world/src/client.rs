use std::{fs::File, io::Read, path::Path};

use zip::ZipArchive;

use crate::WorldError;

const MAX_VERSION_JSON_BYTES: u64 = 1024 * 1024;

/// Reads the data version (`world_version`) from a client JAR's `version.json`.
///
/// Loader instances redirect to a base client JAR, so callers must pass the
/// resolved client JAR path rather than the instance directory.
pub fn client_world_version(client_jar: &Path) -> Result<Option<i32>, WorldError> {
    if !client_jar.is_file() {
        return Ok(None);
    }
    let file = File::open(client_jar).map_err(|source| WorldError::io(client_jar, source))?;
    let mut archive = ZipArchive::new(file).map_err(|source| WorldError::Zip {
        path: client_jar.to_path_buf(),
        source,
    })?;
    let mut entry = match archive.by_name("version.json") {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(source) => {
            return Err(WorldError::Zip {
                path: client_jar.to_path_buf(),
                source,
            });
        }
    };
    let mut bytes = Vec::new();
    entry
        .by_ref()
        .take(MAX_VERSION_JSON_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|source| WorldError::io(client_jar, source))?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| WorldError::ClientJar {
            path: client_jar.to_path_buf(),
            message: error.to_string(),
        })?;
    Ok(value
        .get("world_version")
        .and_then(serde_json::Value::as_i64)
        .and_then(|value| i32::try_from(value).ok()))
}
