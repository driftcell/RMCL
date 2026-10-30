use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorldError {
    #[error("operation was cancelled")]
    Cancelled,
    #[error("invalid world id `{0}`")]
    InvalidWorldId(String),
    #[error("world `{0}` was not found")]
    WorldNotFound(String),
    #[error("world `{path}` is not a valid world directory")]
    InvalidWorld { path: PathBuf },
    #[error("world `{0}` already exists in the target instance")]
    WorldExists(String),
    #[error("world `{0}` is in use by another process")]
    WorldLocked(String),
    #[error("world `{path}` contains a symbolic link; move or import it manually")]
    SymlinkNotAllowed { path: PathBuf },
    #[error("world archive does not contain a level.dat")]
    InvalidArchive,
    #[error("world archive contains more than one world")]
    MultipleWorlds,
    #[error("failed to read `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse `{path}`: {source}")]
    Nbt {
        path: PathBuf,
        #[source]
        source: fastnbt::error::Error,
    },
    #[error("failed to read `{path}` as an NBT file: {source}")]
    Gzip {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read archive `{path}`: {source}")]
    Zip {
        path: PathBuf,
        #[source]
        source: zip::result::ZipError,
    },
    #[error("failed to write archive `{path}`: {source}")]
    ZipWrite {
        path: PathBuf,
        #[source]
        source: zip::result::ZipError,
    },
    #[error("failed to extract world archive: {0}")]
    Extraction(#[from] rmcl_archive::ExtractionError),
    #[error("invalid client jar `{path}`: {message}")]
    ClientJar { path: PathBuf, message: String },
}

impl WorldError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub(crate) fn invalid_path(
        path: impl Into<PathBuf>,
        error: std::path::StripPrefixError,
    ) -> Self {
        Self::Io {
            path: path.into(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, error),
        }
    }
}
