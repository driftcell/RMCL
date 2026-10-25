use thiserror::Error;

#[derive(Debug, Error)]
pub enum MetadataError {
    #[error("invalid Minecraft metadata JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Minecraft version id cannot be blank")]
    BlankVersionId,
}
