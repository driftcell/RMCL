use std::{collections::BTreeMap, path::Path, str::FromStr};

use rmcl_domain::Sha1Hash;
use rmcl_instance::RepositoryPath;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use tempfile::NamedTempFile;
use tokio::{fs, io::AsyncWriteExt};

use crate::{
    JavaError, JavaRequirement, MojangFileType, mojang_platform_key,
    runtime_path::{PathRegistry, normalize_link_target},
};

pub const MARKER_SCHEMA_VERSION: u32 = 1;
const MAX_MARKER_SIZE: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeMarker {
    pub schema_version: u32,
    pub source: String,
    pub platform: String,
    pub component: String,
    pub requested_major: u32,
    pub manifest_url: String,
    pub manifest_sha1: Option<String>,
    pub manifest_size: Option<u64>,
    pub executable: String,
    pub runtime_version: String,
    pub vendor: Option<String>,
    pub installed_at: u64,
    pub files: BTreeMap<String, MarkerFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum MarkerFile {
    File {
        #[serde(default)]
        sha1: Option<String>,
        #[serde(default)]
        size: Option<u64>,
        #[serde(default)]
        executable: bool,
    },
    Directory,
    Link {
        target: String,
    },
}

impl RuntimeMarker {
    pub async fn load(path: &Path) -> Result<Self, JavaError> {
        let bytes = fs::read(path)
            .await
            .map_err(|source| JavaError::ExistingRuntimeInvalid {
                path: path.display().to_string(),
                reason: source.to_string(),
            })?;
        if bytes.len() > MAX_MARKER_SIZE {
            return Err(JavaError::ExistingRuntimeInvalid {
                path: path.display().to_string(),
                reason: "marker exceeds 1 MiB".to_owned(),
            });
        }
        serde_json::from_slice(&bytes).map_err(|source| JavaError::MarkerJson {
            path: path.display().to_string(),
            source,
        })
    }

    pub async fn write(&self, path: &Path) -> Result<(), JavaError> {
        let parent = path.parent().ok_or_else(|| JavaError::MarkerWrite {
            path: path.display().to_string(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "marker parent not found"),
        })?;
        fs::create_dir_all(parent)
            .await
            .map_err(|source| JavaError::MarkerWrite {
                path: path.display().to_string(),
                source,
            })?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|source| JavaError::MarkerJson {
            path: path.display().to_string(),
            source,
        })?;
        let temporary = NamedTempFile::new_in(parent).map_err(|source| JavaError::MarkerWrite {
            path: path.display().to_string(),
            source,
        })?;
        let mut file =
            fs::File::from_std(
                temporary
                    .reopen()
                    .map_err(|source| JavaError::MarkerWrite {
                        path: path.display().to_string(),
                        source,
                    })?,
            );
        file.write_all(&bytes)
            .await
            .map_err(|source| JavaError::MarkerWrite {
                path: path.display().to_string(),
                source,
            })?;
        file.sync_all()
            .await
            .map_err(|source| JavaError::MarkerWrite {
                path: path.display().to_string(),
                source,
            })?;
        drop(file);
        temporary
            .persist(path)
            .map_err(|error| JavaError::MarkerWrite {
                path: path.display().to_string(),
                source: error.error,
            })?;
        Ok(())
    }

    pub async fn validate(
        &self,
        root: &Path,
        requirement: &JavaRequirement,
        component: &str,
    ) -> Result<(), JavaError> {
        if self.schema_version != MARKER_SCHEMA_VERSION {
            return Err(invalid(root, "unsupported marker schema version"));
        }
        if self.source != "mojang" {
            return Err(invalid(root, "unexpected runtime source"));
        }
        let platform =
            mojang_platform_key(requirement.operating_system, &requirement.architecture)?;
        if self.platform != platform || self.component != component {
            return Err(invalid(
                root,
                "platform or component does not match the request",
            ));
        }
        let Some(required_major) = requirement.major_version else {
            return Err(JavaError::MissingMajor);
        };
        if self.requested_major != required_major {
            return Err(invalid(root, "requested major does not match the request"));
        }

        let executable = RepositoryPath::new(&self.executable)
            .map_err(|error| invalid(root, error.to_string()))?;
        let executable_path = executable.to_path(root);
        if !fs::symlink_metadata(&executable_path)
            .await
            .map_err(|source| io_error(root, source))?
            .file_type()
            .is_file()
        {
            return Err(invalid(root, "marker executable is not a regular file"));
        }

        let mut registry = PathRegistry::default();
        for (name, file) in &self.files {
            let path =
                RepositoryPath::new(name).map_err(|error| invalid(root, error.to_string()))?;
            let kind = match file {
                MarkerFile::File { .. } => MojangFileType::File,
                MarkerFile::Directory => MojangFileType::Directory,
                MarkerFile::Link { .. } => MojangFileType::Link,
            };
            registry
                .insert(&path, kind)
                .map_err(|error| invalid(root, error.to_string()))?;
            match file {
                MarkerFile::File {
                    sha1,
                    size,
                    executable,
                } => validate_file(root, &path, *size, sha1.as_deref(), *executable).await?,
                MarkerFile::Directory => {
                    if !fs::symlink_metadata(path.to_path(root))
                        .await
                        .map_err(|source| io_error(root, source))?
                        .file_type()
                        .is_dir()
                    {
                        return Err(invalid(root, "marker directory is not a directory"));
                    }
                }
                MarkerFile::Link { target } => validate_link(root, &path, target).await?,
            }
        }
        Ok(())
    }
}

async fn validate_file(
    root: &Path,
    path: &RepositoryPath,
    expected_size: Option<u64>,
    expected_sha1: Option<&str>,
    executable: bool,
) -> Result<(), JavaError> {
    let full_path = path.to_path(root);
    let metadata = fs::symlink_metadata(&full_path)
        .await
        .map_err(|source| io_error(root, source))?;
    if !metadata.file_type().is_file() {
        return Err(invalid(root, "marker file is not a regular file"));
    }
    let expected_size =
        expected_size.ok_or_else(|| invalid(root, "file size is missing from marker"))?;
    if expected_size != metadata.len() {
        return Err(invalid(root, "file size does not match marker"));
    }
    let expected_sha1 =
        expected_sha1.ok_or_else(|| invalid(root, "file SHA-1 is missing from marker"))?;
    let expected =
        Sha1Hash::from_str(expected_sha1).map_err(|error| invalid(root, error.to_string()))?;
    let actual = hash_file(&full_path).await?;
    if actual != expected {
        return Err(invalid(root, "file SHA-1 does not match marker"));
    }
    #[cfg(not(unix))]
    let _ = executable;
    #[cfg(unix)]
    if executable {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(invalid(root, "executable bit is missing"));
        }
    }
    Ok(())
}

async fn validate_link(
    root: &Path,
    source: &RepositoryPath,
    expected_target: &str,
) -> Result<(), JavaError> {
    let path = source.to_path(root);
    let actual_target = fs::read_link(&path)
        .await
        .map_err(|source| io_error(root, source))?;
    let actual_target = actual_target.to_string_lossy().into_owned();
    let normalized = normalize_link_target(source, &actual_target)
        .map_err(|error| invalid(root, error.to_string()))?;
    if normalized.target.as_str() != expected_target {
        return Err(invalid(root, "link target does not match marker"));
    }
    let target_path = normalized.target.to_path(root);
    let metadata = fs::symlink_metadata(&target_path)
        .await
        .map_err(|source| io_error(root, source))?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(JavaError::MissingLinkTarget(
            target_path.display().to_string(),
        ));
    }
    Ok(())
}

async fn hash_file(path: &Path) -> Result<Sha1Hash, JavaError> {
    let mut file = fs::File::open(path)
        .await
        .map_err(|source| io_error(path, source))?;
    let mut hasher = Sha1::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = tokio::io::AsyncReadExt::read(&mut file, &mut buffer)
            .await
            .map_err(|source| io_error(path, source))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 20];
    bytes.copy_from_slice(&digest);
    Ok(Sha1Hash::from_bytes(bytes))
}

fn invalid(root: &Path, reason: impl Into<String>) -> JavaError {
    JavaError::ExistingRuntimeInvalid {
        path: root.display().to_string(),
        reason: reason.into(),
    }
}

fn io_error(path: &Path, source: std::io::Error) -> JavaError {
    JavaError::Io {
        path: path.display().to_string(),
        source,
    }
}
