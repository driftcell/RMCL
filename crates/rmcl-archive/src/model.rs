use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ExtractionLimits {
    pub max_entries: usize,
    pub max_entry_size: u64,
    pub max_total_size: u64,
}

impl Default for ExtractionLimits {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            max_entry_size: 256 * 1024 * 1024,
            max_total_size: 1024 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionProgress {
    pub entry: String,
    pub completed_entries: usize,
    pub total_entries: usize,
    pub extracted_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractionReport {
    pub destination: PathBuf,
    pub files: u64,
    pub directories: u64,
    pub extracted_bytes: u64,
    pub skipped: u64,
}
