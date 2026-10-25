//! Minecraft metadata models and headless compatibility rules.

mod asset;
mod download;
mod error;
mod library;
mod manifest;
mod rules;
mod version;

pub use asset::{AssetCatalog, AssetObject};
pub use download::DownloadInfo;
pub use error::MetadataError;
pub use library::{ExtractRules, Library, LibraryDownloads};
pub use manifest::{LatestVersions, VersionManifest, VersionSummary};
pub use rmcl_domain::{Architecture, OperatingSystem, ParseSha1Error, Platform, Sha1Hash};
pub use rules::{FeatureMap, Rule, RuleAction, RuleError, evaluate_rules};
pub use version::{
    Argument, Arguments, AssetIndex, GameVersion, JavaVersion, RuledArgument, evaluate_arguments,
};
