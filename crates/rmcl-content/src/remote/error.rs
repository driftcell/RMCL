use thiserror::Error;

use super::ContentSource;

/// Errors from remote content metadata requests.
#[derive(Debug, Error)]
pub enum RemoteContentError {
    #[error("invalid remote content configuration: {message}")]
    Configuration { message: String },
    #[error("request to {provider} failed: {source}")]
    Http {
        provider: ContentSource,
        #[source]
        source: Box<reqwest::Error>,
    },
    #[error("{provider} returned HTTP {status}: {message}")]
    Api {
        provider: ContentSource,
        status: u16,
        message: String,
    },
    #[error("invalid response from {provider}: {source}")]
    Json {
        provider: ContentSource,
        #[source]
        source: Box<serde_json::Error>,
    },
    #[error("invalid response from {provider}: {message}")]
    InvalidResponse {
        provider: ContentSource,
        message: String,
    },
    #[error("remote content request was cancelled")]
    Cancelled,
}
