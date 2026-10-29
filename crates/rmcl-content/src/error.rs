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
    #[error("invalid {kind} metadata in `{path}`: {reason}")]
    PackMetadata {
        kind: ContentKind,
        path: PathBuf,
        reason: String,
    },
    #[error("resource pack `{file_name}` is not enabled")]
    ResourcePackNotEnabled { file_name: String },
    #[error("invalid Minecraft client metadata in `{path}`: {reason}")]
    ClientMetadata { path: PathBuf, reason: String },
    #[error("invalid shader configuration `{path}`: {reason}")]
    ShaderConfig { path: PathBuf, reason: String },
    #[error("cannot {operation} for {kind}")]
    UnsupportedOperation {
        kind: ContentKind,
        operation: &'static str,
    },
    #[error(transparent)]
    Layout(#[from] rmcl_instance::LayoutError),
}
