use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionPlan {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub exclude_prefixes: Vec<String>,
}
