use std::{collections::HashMap, path::PathBuf};

use rmcl_domain::{DownloadRequest, ExtractionPlan};
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

impl From<PlannedFile> for DownloadRequest {
    fn from(file: PlannedFile) -> Self {
        Self {
            sources: file.sources,
            destination: file.destination,
            sha1: file.sha1,
            size: file.size,
        }
    }
}

impl From<&PlannedFile> for DownloadRequest {
    fn from(file: &PlannedFile) -> Self {
        Self {
            sources: file.sources.clone(),
            destination: file.destination.clone(),
            sha1: file.sha1,
            size: file.size,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlannedFileKind {
    Client,
    Library,
    NativeLibrary,
    AssetIndex,
    AssetObject,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_planned_files_to_download_requests() {
        let hash = "0123456789abcdef0123456789abcdef01234567".parse().unwrap();
        let file = PlannedFile {
            kind: PlannedFileKind::Client,
            destination: PathBuf::from("versions/example/example.jar"),
            sources: vec![Url::parse("https://example.invalid/client.jar").unwrap()],
            sha1: Some(hash),
            size: Some(42),
        };

        let request = DownloadRequest::from(&file);
        assert_eq!(request.destination, file.destination);
        assert_eq!(request.sources, file.sources);
        assert_eq!(request.sha1, Some(hash));
        assert_eq!(request.size, Some(42));

        let owned = DownloadRequest::from(file);
        assert_eq!(owned, request);
    }
}
