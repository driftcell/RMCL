use std::path::PathBuf;

use thiserror::Error;

use crate::ContentKind;

/// Errors from local content scanning and mutation.
#[derive(Debug, Error)]
pub enum ContentError {
    #[error("failed to {operation} `{path}`: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("`{path}` is not a supported {kind} file")]
    UnsupportedFile { kind: ContentKind, path: PathBuf },
    #[error("content entry `{path}` already exists")]
    Conflict { path: PathBuf },
    #[error("content entry `{path}` was not found")]
    NotFound { path: PathBuf },
    #[error("content entry `{path}` is a symbolic link and cannot be managed")]
    Symlink { path: PathBuf },
    #[error("content entry `{path}` is not a directory")]
    NotADirectory { path: PathBuf },
    #[error("unsafe content path `{path}`: {reason}")]
    UnsafePath { path: PathBuf, reason: String },
    #[error("resource pack options file `{path}` is invalid: {reason}")]
    Options { path: PathBuf, reason: String },
    #[error("cannot {operation} for {kind}")]
    UnsupportedOperation {
        kind: ContentKind,
        operation: &'static str,
    },
    #[error(transparent)]
    Layout(#[from] rmcl_instance::LayoutError),
}
