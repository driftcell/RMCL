use rmcl_domain::Architecture;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum JavaError {
    #[error("Java executable was not found: `{0}`")]
    NotFound(String),
    #[error("failed to start Java probe `{path}`: {source}")]
    Spawn {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("Java probe `{0}` timed out")]
    ProbeTimeout(String),
    #[error("Java probe `{0}` produced too much output")]
    OutputTooLarge(String),
    #[error("Java probe `{path}` exited with {status}: {output}")]
    ProbeFailed {
        path: String,
        status: String,
        output: String,
    },
    #[error("Java probe `{path}` did not expose `{stream}`")]
    MissingOutput { path: String, stream: &'static str },
    #[error("Java probe `{path}` is missing `{field}`")]
    MissingProperty { path: String, field: &'static str },
    #[error("unsupported Java version `{version}` from `{path}`")]
    InvalidVersion { path: String, version: String },
    #[error("unsupported operating system `{os}` from `{path}`")]
    UnsupportedOperatingSystem { path: String, os: String },
    #[error("unsupported Java architecture `{architecture}` from `{path}`")]
    UnsupportedArchitecture { path: String, architecture: String },
    #[error("no Java major requirement was provided")]
    MissingMajor,
    #[error("automatic Java installation is unsupported on {os:?} with {architecture:?}")]
    UnsupportedPlatform {
        os: rmcl_domain::OperatingSystem,
        architecture: Architecture,
    },
    #[error("no canonical Mojang runtime component exists for Java {major}")]
    UnsupportedMajor { major: u32 },
    #[error("no Mojang runtime for Java {major} on {platform}; tried components {components:?}")]
    NoRuntimeFor {
        platform: String,
        major: u32,
        components: Vec<String>,
    },
    #[error("invalid runtime download URL `{url}`: {source}")]
    InvalidUrl {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("runtime download host is not allowed: `{0}`")]
    UnexpectedHost(String),
    #[error(transparent)]
    Checksum(#[from] rmcl_domain::ParseSha1Error),
    #[error("runtime metadata is missing SHA-1 or size for `{0}`")]
    MissingIntegrity(String),
    #[error("runtime HTTP request for `{url}` failed: {source}")]
    Http {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("runtime HTTP request for `{url}` returned {status}")]
    HttpStatus {
        url: String,
        status: reqwest::StatusCode,
    },
    #[error("runtime manifest is larger than {limit} bytes")]
    ManifestTooLarge { limit: u64 },
    #[error("runtime manifest size mismatch: expected {expected}, got {actual}")]
    ManifestSizeMismatch { expected: u64, actual: u64 },
    #[error("runtime manifest SHA-1 mismatch: expected {expected}, got {actual}")]
    ManifestHashMismatch {
        expected: rmcl_domain::Sha1Hash,
        actual: rmcl_domain::Sha1Hash,
    },
    #[error("invalid runtime manifest `{path}`: {source}")]
    ManifestJson {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("runtime file `{0}` has no raw download available")]
    MissingRawDownload(String),
    #[error("runtime downloads failed: {failures:?}")]
    DownloadFailures { failures: Vec<String> },
    #[error("runtime path `{path}` conflicts with `{existing}`")]
    PathConflict { path: String, existing: String },
    #[error("invalid runtime manifest entry `{path}`: {reason}")]
    InvalidManifest { path: String, reason: String },
    #[error("invalid runtime link `{path}`: {reason}")]
    InvalidLink { path: String, reason: String },
    #[error("runtime link target does not exist or is not a regular file/directory: `{0}`")]
    MissingLinkTarget(String),
    #[error("failed to create runtime link `{path}`: {source}")]
    LinkCreation {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("another installation is already running for `{0}`")]
    AlreadyInstalling(String),
    #[error("existing runtime at `{path}` is invalid: {reason}")]
    ExistingRuntimeInvalid { path: String, reason: String },
    #[error("no Java executable found under `{0}`")]
    MissingJavaExecutable(String),
    #[error("runtime probe for `{path}` was rejected: {reason}")]
    ProbeRejected { path: String, reason: String },
    #[error("failed to publish runtime at `{path}`: {source}")]
    Publish {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize runtime marker at `{path}`: {source}")]
    MarkerJson {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write runtime marker at `{path}`: {source}")]
    MarkerWrite {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Path(#[from] rmcl_instance::LayoutError),
    #[error(transparent)]
    Download(#[from] rmcl_net::DownloadError),
    #[error("I/O failed for `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}
