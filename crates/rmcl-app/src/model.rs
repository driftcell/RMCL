use std::path::PathBuf;

use rmcl_archive::ExtractionReport;
use rmcl_domain::Platform;
use rmcl_install::InstallPlan;
use rmcl_instance::{InstanceLayout, MinecraftLayout};
use rmcl_minecraft::GameVersion;
use rmcl_net::{DownloadReport, DownloadStatus, ProgressStage};
use tokio::sync::mpsc;
use url::Url;

#[derive(Debug, Clone)]
pub struct VanillaSources {
    pub manifest_url: Url,
    pub asset_object_base: Url,
}

impl VanillaSources {
    pub fn official() -> Result<Self, url::ParseError> {
        Ok(Self {
            manifest_url: Url::parse(
                "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json",
            )?,
            asset_object_base: Url::parse("https://resources.download.minecraft.net/")?,
        })
    }
}

#[derive(Debug, Clone)]
pub enum VersionSource {
    Manifest,
    LocalJson(PathBuf),
    Existing,
}

#[derive(Debug, Clone)]
pub struct VanillaInstallRequest {
    pub minecraft_root: PathBuf,
    pub version_id: String,
    pub platform: Platform,
    pub source: VersionSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallPhase {
    ResolvingVersion,
    ReadingVersion,
    PersistingVersion,
    PlanningBase,
    DownloadingBase,
    ReadingAssetIndex,
    PlanningAssets,
    DownloadingAssets,
    MergingNatives,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallDownloadProgress {
    pub phase: InstallPhase,
    pub stage: ProgressStage,
    pub current: PathBuf,
    pub completed_files: usize,
    pub total_files: usize,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallEvent {
    Phase(InstallPhase),
    Download(InstallDownloadProgress),
    Natives {
        current: PathBuf,
        completed_archives: usize,
        total_archives: usize,
    },
}

pub type InstallEventSender = mpsc::Sender<InstallEvent>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DownloadSummary {
    pub downloaded_files: u64,
    pub skipped_files: u64,
    pub downloaded_bytes: u64,
    pub skipped_bytes: u64,
}

impl DownloadSummary {
    pub fn total_files(self) -> u64 {
        self.downloaded_files + self.skipped_files
    }

    pub(crate) fn from_report(report: &DownloadReport) -> Self {
        let mut summary = Self::default();
        for outcome in &report.outcomes {
            match outcome.status {
                DownloadStatus::Downloaded { bytes } => {
                    summary.downloaded_files += 1;
                    summary.downloaded_bytes += bytes;
                }
                DownloadStatus::Skipped { bytes } => {
                    summary.skipped_files += 1;
                    summary.skipped_bytes += bytes;
                }
            }
        }
        summary
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.downloaded_files += other.downloaded_files;
        self.skipped_files += other.skipped_files;
        self.downloaded_bytes += other.downloaded_bytes;
        self.skipped_bytes += other.skipped_bytes;
    }
}

#[derive(Debug, Clone)]
pub struct InstallOutcome {
    pub layout: MinecraftLayout,
    /// Instance paths and game-directory semantics resolved at install time.
    pub instance: InstanceLayout,
    pub platform: Platform,
    pub version: GameVersion,
    pub version_json_path: PathBuf,
    pub base_plan: InstallPlan,
    pub asset_plan: InstallPlan,
    pub downloads: DownloadSummary,
    pub natives: Vec<ExtractionReport>,
}
