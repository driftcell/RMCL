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
    #[error("I/O failed for `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}
