//! A local library of skins imported by the user.
//!
//! The official service has no documented "activate owned skin by id" API, so
//! RMCL keeps a small local copy for quick re-upload. Entries are keyed by the
//! upload bytes, which also makes repeated imports idempotent.

use std::{
    io::{Cursor, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use image::{DynamicImage, ImageFormat};
use rmcl_auth::SkinVariant;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::skin_source::{PreparedSkin, SkinSourceError, prepare_skin};

const INDEX_FILE: &str = "index.json";
const SCHEMA_VERSION: u32 = 1;

/// Metadata for one locally stored skin.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkinLibraryEntry {
    pub id: String,
    pub name: String,
    pub variant: SkinVariant,
    pub source_url: Option<String>,
    pub imported_at: u64,
    pub file_name: String,
}

/// A user-owned skin library rooted in the launcher configuration directory.
#[derive(Clone)]
pub struct SkinLibrary {
    root: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl SkinLibrary {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Lists entries newest first.
    pub async fn list(&self) -> Result<Vec<SkinLibraryEntry>, SkinLibraryError> {
        let _guard = self.lock.lock().await;
        let mut index = read_index(&self.root).await?;
        index
            .entries
            .sort_by_key(|entry| std::cmp::Reverse(entry.imported_at));
        Ok(index.entries)
    }

    /// Stores a validated skin and returns its entry.
    pub async fn import(&self, skin: &PreparedSkin) -> Result<SkinLibraryEntry, SkinLibraryError> {
        let _guard = self.lock.lock().await;
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(|source| SkinLibraryError::Io {
                operation: "create skin library directory",
                path: self.root.clone(),
                source,
            })?;

        let id = skin.id().to_owned();
        let path = self.skin_path(&id);
        write_atomic(&path, skin.bytes()).await?;
        let mut head = Vec::new();
        DynamicImage::ImageRgba8(skin.texture().head_image())
            .write_to(&mut Cursor::new(&mut head), ImageFormat::Png)
            .map_err(SkinLibraryError::Encode)?;
        write_atomic(&self.head_path_unchecked(&id), &head).await?;

        let mut index = read_index(&self.root).await?;
        let entry = SkinLibraryEntry {
            id: id.clone(),
            name: skin.file_name().to_owned(),
            variant: skin.variant_hint(),
            source_url: skin.source_url().map(str::to_owned),
            imported_at: unix_timestamp(),
            file_name: skin.file_name().to_owned(),
        };
        index.entries.retain(|existing| existing.id != id);
        index.entries.push(entry.clone());
        write_index(&self.root, &index).await?;
        Ok(entry)
    }

    /// Loads and re-validates a stored skin.
    pub async fn load(&self, id: &str) -> Result<PreparedSkin, SkinLibraryError> {
        let _guard = self.lock.lock().await;
        let index = read_index(&self.root).await?;
        let entry = index
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| SkinLibraryError::NotFound { id: id.to_owned() })?;
        validate_id(&entry.id)?;
        let path = self.skin_path(&entry.id);
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|source| SkinLibraryError::Io {
                operation: "read skin library file",
                path,
                source,
            })?;
        prepare_skin(&bytes, entry.file_name.clone(), entry.source_url.clone())
            .map_err(SkinLibraryError::Source)
    }

    /// Renames an entry without changing its bytes.
    pub async fn rename(&self, id: &str, name: impl Into<String>) -> Result<(), SkinLibraryError> {
        let _guard = self.lock.lock().await;
        let mut index = read_index(&self.root).await?;
        let entry = index
            .entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| SkinLibraryError::NotFound { id: id.to_owned() })?;
        entry.name = name.into();
        write_index(&self.root, &index).await
    }

    /// Removes an entry and its file.
    pub async fn remove(&self, id: &str) -> Result<(), SkinLibraryError> {
        let _guard = self.lock.lock().await;
        validate_id(id)?;
        let mut index = read_index(&self.root).await?;
        let before = index.entries.len();
        index.entries.retain(|entry| entry.id != id);
        if index.entries.len() == before {
            return Err(SkinLibraryError::NotFound { id: id.to_owned() });
        }
        let path = self.skin_path(id);
        match tokio::fs::remove_file(&path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(SkinLibraryError::Io {
                    operation: "remove skin library file",
                    path,
                    source,
                });
            }
        }
        write_index(&self.root, &index).await
    }

    /// Returns the cached avatar path for a library entry.
    pub fn head_path(&self, id: &str) -> Option<PathBuf> {
        validate_id(id).ok().map(|_| self.head_path_unchecked(id))
    }

    fn skin_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.png"))
    }

    fn head_path_unchecked(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.head.png"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SkinIndex {
    schema_version: u32,
    #[serde(default)]
    entries: Vec<SkinLibraryEntry>,
}

impl Default for SkinIndex {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }
}

async fn read_index(root: &Path) -> Result<SkinIndex, SkinLibraryError> {
    let path = root.join(INDEX_FILE);
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SkinIndex::default());
        }
        Err(source) => {
            return Err(SkinLibraryError::Io {
                operation: "read skin library index",
                path,
                source,
            });
        }
    };
    let index: SkinIndex = serde_json::from_slice(&bytes).map_err(SkinLibraryError::Index)?;
    if index.schema_version != SCHEMA_VERSION {
        return Err(SkinLibraryError::UnsupportedSchema {
            found: index.schema_version,
        });
    }
    Ok(index)
}

async fn write_index(root: &Path, index: &SkinIndex) -> Result<(), SkinLibraryError> {
    let bytes = serde_json::to_vec_pretty(index).map_err(SkinLibraryError::Serialize)?;
    write_atomic(&root.join(INDEX_FILE), &bytes).await
}

async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), SkinLibraryError> {
    let path = path.to_path_buf();
    let parent = path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| SkinLibraryError::Io {
            operation: "find skin library parent",
            path: path.clone(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "skin library path has no parent",
            ),
        })?;
    tokio::fs::create_dir_all(&parent)
        .await
        .map_err(|source| SkinLibraryError::Io {
            operation: "create skin library directory",
            path: parent.clone(),
            source,
        })?;
    let bytes = bytes.to_vec();
    tokio::task::spawn_blocking(move || {
        let mut temporary =
            NamedTempFile::new_in(&parent).map_err(|source| SkinLibraryError::Io {
                operation: "create skin library temporary file",
                path: path.clone(),
                source,
            })?;
        temporary
            .write_all(&bytes)
            .map_err(|source| SkinLibraryError::Io {
                operation: "write skin library file",
                path: temporary.path().to_path_buf(),
                source,
            })?;
        temporary
            .persist(&path)
            .map_err(|error| SkinLibraryError::Io {
                operation: "publish skin library file",
                path,
                source: error.error,
            })?;
        Ok::<_, SkinLibraryError>(())
    })
    .await
    .map_err(|error| SkinLibraryError::Blocking(error.to_string()))?
}

fn validate_id(id: &str) -> Result<(), SkinLibraryError> {
    if id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(SkinLibraryError::InvalidId { id: id.to_owned() })
    }
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug, Error)]
pub enum SkinLibraryError {
    #[error("failed to {operation} `{}`: {source}", path.display())]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("skin library index is invalid: {0}")]
    Index(#[source] serde_json::Error),
    #[error("failed to serialize skin library index: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("failed to encode skin library head: {0}")]
    Encode(#[source] image::ImageError),
    #[error("skin library uses unsupported schema {found}")]
    UnsupportedSchema { found: u32 },
    #[error("skin `{id}` was not found in the local library")]
    NotFound { id: String },
    #[error("invalid skin library id `{id}`")]
    InvalidId { id: String },
    #[error(transparent)]
    Source(#[from] SkinSourceError),
    #[error("skin library task failed: {0}")]
    Blocking(String),
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    use tempfile::TempDir;

    use super::*;

    fn prepared(name: &str, color: [u8; 4]) -> PreparedSkin {
        let image = RgbaImage::from_pixel(64, 64, Rgba(color));
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(image)
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        prepare_skin(
            &bytes,
            name,
            Some("https://example.com/skin.png".to_owned()),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn imports_lists_loads_and_removes() {
        let temporary = TempDir::new().unwrap();
        let library = SkinLibrary::new(temporary.path().join("skins"));
        let skin = prepared("my-skin.png", [20, 40, 60, 255]);

        let entry = library.import(&skin).await.unwrap();
        assert_eq!(entry.name, "my-skin.png");
        assert!(
            temporary
                .path()
                .join("skins")
                .join(format!("{}.png", entry.id))
                .exists()
        );

        let entries = library.list().await.unwrap();
        assert_eq!(entries, vec![entry.clone()]);

        let loaded = library.load(&entry.id).await.unwrap();
        assert_eq!(loaded.texture().dimensions(), (64, 64));
        assert_eq!(loaded.source_url(), Some("https://example.com/skin.png"));

        library.remove(&entry.id).await.unwrap();
        assert!(library.list().await.unwrap().is_empty());
        assert!(matches!(
            library.load(&entry.id).await.unwrap_err(),
            SkinLibraryError::NotFound { .. }
        ));
    }

    #[tokio::test]
    async fn duplicate_import_updates_metadata_without_duplicating() {
        let temporary = TempDir::new().unwrap();
        let library = SkinLibrary::new(temporary.path().join("skins"));
        let skin = prepared("first.png", [10, 20, 30, 255]);

        let first = library.import(&skin).await.unwrap();
        let second = library.import(&skin).await.unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(library.list().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn rejects_invalid_ids_and_unsupported_schema() {
        let temporary = TempDir::new().unwrap();
        let library = SkinLibrary::new(temporary.path().join("skins"));
        assert!(matches!(
            library.remove("../index").await.unwrap_err(),
            SkinLibraryError::InvalidId { .. }
        ));

        tokio::fs::create_dir_all(library.root()).await.unwrap();
        tokio::fs::write(
            library.root().join(INDEX_FILE),
            br#"{"schema_version": 99, "entries": []}"#,
        )
        .await
        .unwrap();
        assert!(matches!(
            library.list().await.unwrap_err(),
            SkinLibraryError::UnsupportedSchema { found: 99 }
        ));
    }
}
