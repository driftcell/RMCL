//! Minecraft metadata models and headless compatibility rules.

mod download;
mod error;
mod library;
mod manifest;
mod rules;
mod version;

pub use download::DownloadInfo;
pub use error::MetadataError;
pub use library::{ExtractRules, Library, LibraryDownloads};
pub use manifest::{LatestVersions, VersionManifest, VersionSummary};
pub use rules::{
    FeatureMap, OperatingSystem, Platform, Rule, RuleAction, RuleError, evaluate_rules,
};
pub use version::{
    Argument, Arguments, AssetIndex, GameVersion, JavaVersion, RuledArgument, evaluate_arguments,
};
