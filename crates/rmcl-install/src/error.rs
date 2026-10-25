use rmcl_instance::LayoutError;
use rmcl_minecraft::{ParseSha1Error, RuleError};
use thiserror::Error;

use crate::MavenCoordinateError;

#[derive(Debug, Error)]
pub enum InstallPlanError {
    #[error(transparent)]
    Layout(#[from] LayoutError),
    #[error(transparent)]
    Checksum(#[from] ParseSha1Error),
    #[error(transparent)]
    Coordinate(#[from] MavenCoordinateError),
    #[error(transparent)]
    Rule(#[from] RuleError),
    #[error("version `{id}` redirects its client JAR to unsupported version `{jar}`")]
    UnsupportedClientJarRedirect { id: String, jar: String },
    #[error("version `{0}` has no client download")]
    MissingClientDownload(String),
    #[error("version `{0}` has no asset index")]
    MissingAssetIndex(String),
    #[error("library `{0}` has no artifact metadata")]
    MissingLibraryArtifact(String),
    #[error("library `{library}` has no native classifier `{classifier}` in downloads.classifiers")]
    MissingNativeClassifier { library: String, classifier: String },
    #[error("native classifier `{classifier}` for `{library}` contains an unresolved placeholder")]
    UnresolvedNativeClassifier { library: String, classifier: String },
    #[error(
        "native classifier `{classifier}` for `{library}` requires a known 32/64-bit target architecture"
    )]
    UnknownNativeArchitecture { library: String, classifier: String },
    #[error("download for `{artifact}` has no URL")]
    MissingDownloadUrl { artifact: String },
    #[error("invalid download URL `{url}` for `{artifact}`: {source}")]
    InvalidUrl {
        artifact: String,
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("download URL `{url}` for `{artifact}` cannot be used as a base URL")]
    UrlCannotBeBase { artifact: String, url: String },
    #[error("unsupported download URL scheme `{scheme}` for `{artifact}`")]
    UnsupportedUrlScheme { artifact: String, scheme: String },
    #[error("conflicting download requirements for `{path}`")]
    ConflictingDownload { path: String },
}
