use thiserror::Error;

#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("download concurrency and attempts must be greater than zero")]
    InvalidConfiguration,
    #[error("host concurrency limiter is unavailable")]
    ConcurrencyLimiterClosed,
    #[error("download request has no source URLs")]
    EmptySources,
    #[error("download destination is invalid: `{0}`")]
    InvalidDestination(String),
    #[error("multiple requests target the same destination `{0}`")]
    ConflictingDestination(String),
    #[error("download was cancelled")]
    Cancelled,
    #[error("I/O failed for `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to build HTTP client: {0}")]
    HttpClient(#[from] reqwest::Error),
    #[error("all sources failed for `{destination}`: {failures:?}")]
    Exhausted {
        destination: String,
        failures: Vec<SourceFailure>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFailure {
    pub source: String,
    pub reason: String,
}
