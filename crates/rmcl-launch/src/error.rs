use thiserror::Error;

#[derive(Debug, Error)]
pub enum LaunchError {
    #[error("game version `{0}` has no main class")]
    MissingMainClass(String),
    #[error("Java runtime `{path}` does not satisfy game requirements: {reason}")]
    IncompatibleJava { path: String, reason: String },
    #[error("invalid legacy arguments: {0}")]
    InvalidLegacyArguments(String),
    #[error("invalid launch arguments: {0}")]
    InvalidArguments(String),
    #[error("invalid classpath: {0}")]
    InvalidClasspath(String),
    #[error("authlib-injector jar is missing: `{path}`")]
    AuthlibInjectorMissing { path: String },
    #[error("authlib-injector jar path must not contain `=`: `{path}`")]
    AuthlibInjectorPathContainsEquals { path: String },
    #[error("authlib-injector API root cannot be blank")]
    AuthlibApiRootBlank,
    #[error("launch path is not valid UTF-8: `{0}`")]
    NonUtf8Path(String),
    #[error("failed to resolve launch path `{path}` as an absolute path: {source}")]
    AbsolutePath {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to spawn game process `{path}`: {source}")]
    Spawn {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("game process I/O failed for `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("game process task failed: {0}")]
    ProcessTask(String),
    #[error(transparent)]
    Path(#[from] rmcl_instance::LayoutError),
    #[error(transparent)]
    Rule(#[from] rmcl_minecraft::RuleError),
}
