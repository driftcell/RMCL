use std::path::PathBuf;

use rmcl_content::{
    ConflictPolicy, ContentKind, ContentLibrary, LocalContent, delete_content, import_file,
    scan_library, set_content_enabled,
};
use rmcl_instance::{InstanceLayout, RepositoryPath};

use crate::AppError;

/// Scans all local content directories of one instance.
pub async fn scan_instance_content(instance: InstanceLayout) -> Result<ContentLibrary, AppError> {
    tokio::task::spawn_blocking(move || scan_library(&instance))
        .await
        .map_err(|error| AppError::BlockingTask(error.to_string()))
}

/// Imports one local file into the selected instance content directory.
pub async fn import_instance_content(
    instance: InstanceLayout,
    kind: ContentKind,
    source: PathBuf,
    policy: ConflictPolicy,
) -> Result<LocalContent, AppError> {
    tokio::task::spawn_blocking(move || import_file(&instance, kind, source, policy))
        .await
        .map_err(|error| AppError::BlockingTask(error.to_string()))?
        .map_err(AppError::Content)
}

/// Deletes one local content entry from an instance.
pub async fn delete_instance_content(
    instance: InstanceLayout,
    kind: ContentKind,
    relative_path: RepositoryPath,
) -> Result<(), AppError> {
    tokio::task::spawn_blocking(move || delete_content(&instance, kind, &relative_path))
        .await
        .map_err(|error| AppError::BlockingTask(error.to_string()))?
        .map_err(AppError::Content)
}

/// Enables or disables one mod or resource pack.
pub async fn set_instance_content_enabled(
    instance: InstanceLayout,
    kind: ContentKind,
    relative_path: RepositoryPath,
    enabled: bool,
) -> Result<LocalContent, AppError> {
    tokio::task::spawn_blocking(move || {
        set_content_enabled(&instance, kind, &relative_path, enabled)
    })
    .await
    .map_err(|error| AppError::BlockingTask(error.to_string()))?
    .map_err(AppError::Content)
}
