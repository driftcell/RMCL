use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("failed to {operation} `{path}`: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid launcher configuration `{path}`: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("unsupported configuration schema {found}; this build supports schema {supported}")]
    UnsupportedSchema { found: u32, supported: u32 },
}
