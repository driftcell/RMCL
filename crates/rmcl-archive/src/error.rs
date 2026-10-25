use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExtractionError {
    #[error("archive source is not a regular file: `{0}`")]
    InvalidSource(String),
    #[error("extraction destination is invalid: `{0}`")]
    InvalidDestination(String),
    #[error("extraction destination already exists: `{0}`")]
    DestinationExists(String),
    #[error("extraction was cancelled")]
    Cancelled,
    #[error("ZIP error for `{path}`: {source}")]
    Zip {
        path: String,
        #[source]
        source: zip::result::ZipError,
    },
    #[error("I/O failed for `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("unsafe archive entry `{entry}`: {reason}")]
    UnsafeEntry { entry: String, reason: &'static str },
    #[error("unsupported archive entry type `{entry}`")]
    UnsupportedEntry { entry: String },
    #[error("duplicate or platform-conflicting archive entry `{entry}`")]
    DuplicateEntry { entry: String },
    #[error("archive contains {actual} entries, exceeding the limit of {limit}")]
    TooManyEntries { actual: usize, limit: usize },
    #[error("archive entry `{entry}` has size {actual}, exceeding the limit of {limit}")]
    EntryTooLarge {
        entry: String,
        actual: u64,
        limit: u64,
    },
    #[error("archive expanded size {actual} exceeds the limit of {limit}")]
    TotalTooLarge { actual: u64, limit: u64 },
    #[error("archive entry `{entry}` declared {expected} bytes but produced {actual}")]
    EntrySizeMismatch {
        entry: String,
        expected: u64,
        actual: u64,
    },
    #[error("archive entry `{entry}` failed CRC-32 validation")]
    CrcMismatch { entry: String },
}
