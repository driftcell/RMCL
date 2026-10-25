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
    #[error("I/O failed for `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}
