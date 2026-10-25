use std::{collections::HashMap, path::PathBuf};

use rmcl_minecraft::Sha1Hash;
use url::Url;

use crate::InstallPlanError;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallPlan {
    pub files: Vec<PlannedFile>,
    pub extractions: Vec<ExtractionPlan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFile {
    pub kind: PlannedFileKind,
    pub destination: PathBuf,
    pub sources: Vec<Url>,
    pub sha1: Option<Sha1Hash>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlannedFileKind {
    Client,
    Library,
    NativeLibrary,
    AssetIndex,
    AssetObject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionPlan {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub exclude_prefixes: Vec<String>,
}

#[derive(Default)]
pub(crate) struct InstallPlanBuilder {
    plan: InstallPlan,
    file_indices: HashMap<PathBuf, usize>,
}

impl InstallPlanBuilder {
    pub(crate) fn push_file(&mut self, file: PlannedFile) -> Result<(), InstallPlanError> {
        if let Some(index) = self.file_indices.get(&file.destination).copied() {
            let existing = &mut self.plan.files[index];
            if existing.sha1 != file.sha1 || existing.size != file.size {
                return Err(conflict(&file));
            }

            let has_new_source = file
                .sources
                .iter()
                .any(|source| !existing.sources.contains(source));
            if existing.sha1.is_none() && has_new_source {
                return Err(conflict(&file));
            }

            for source in file.sources {
                if !existing.sources.contains(&source) {
                    existing.sources.push(source);
                }
            }
            return Ok(());
        }

        self.file_indices
            .insert(file.destination.clone(), self.plan.files.len());
        self.plan.files.push(file);
        Ok(())
    }

    pub(crate) fn push_extraction(&mut self, extraction: ExtractionPlan) {
        if !self.plan.extractions.contains(&extraction) {
            self.plan.extractions.push(extraction);
        }
    }

    pub(crate) fn finish(self) -> InstallPlan {
        self.plan
    }
}

fn conflict(file: &PlannedFile) -> InstallPlanError {
    InstallPlanError::ConflictingDownload {
        path: file.destination.display().to_string(),
    }
}
