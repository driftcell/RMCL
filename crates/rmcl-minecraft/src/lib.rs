//! Minecraft metadata models and headless compatibility rules.

mod asset;
mod checksum;
mod download;
mod error;
mod library;
mod manifest;
mod rules;
mod version;

pub use asset::{AssetCatalog, AssetObject};
pub use checksum::{ParseSha1Error, Sha1Hash};
pub use download::DownloadInfo;
pub use error::MetadataError;
pub use library::{ExtractRules, Library, LibraryDownloads};
pub use manifest::{LatestVersions, VersionManifest, VersionSummary};
pub use rules::{
    Architecture, FeatureMap, OperatingSystem, Platform, Rule, RuleAction, RuleError,
    evaluate_rules,
};
pub use version::{
    Argument, Arguments, AssetIndex, GameVersion, JavaVersion, RuledArgument, evaluate_arguments,
};
