use std::fmt::Display;

use rmcl_app::AppError;
use rmcp::ErrorData;
use serde::Serialize;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum McpServerError {
    #[error("failed to bind MCP server: {0}")]
    Bind(#[from] std::io::Error),
    #[error("MCP server failed: {0}")]
    Serve(String),
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidInput,
    NotFound,
    AlreadyExists,
    RevisionConflict,
    TaskAlreadyRunning,
    RepositoryBusy,
    PermissionDenied,
    Conflict,
    Cancelled,
    Unavailable,
    InternalError,
}

#[derive(Debug, Serialize)]
pub struct ErrorDetails {
    pub code: ErrorCode,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl ErrorDetails {
    fn new(code: ErrorCode, retryable: bool) -> Self {
        Self {
            code,
            retryable,
            expected_revision: None,
            actual_revision: None,
            task_id: None,
            path: None,
        }
    }
}

fn invalid_with_details(message: impl Into<String>, details: ErrorDetails) -> ErrorData {
    ErrorData::invalid_params(message.into(), Some(json!(details)))
}

fn internal_with_details(message: impl Into<String>, details: ErrorDetails) -> ErrorData {
    ErrorData::internal_error(message.into(), Some(json!(details)))
}

pub fn internal_error(error: impl Display) -> ErrorData {
    internal_with_details(
        error.to_string(),
        ErrorDetails::new(ErrorCode::InternalError, false),
    )
}

pub fn invalid_input(message: impl Into<String>) -> ErrorData {
    invalid_with_details(message, ErrorDetails::new(ErrorCode::InvalidInput, false))
}

pub fn not_found(message: impl Into<String>) -> ErrorData {
    invalid_with_details(message, ErrorDetails::new(ErrorCode::NotFound, false))
}

pub fn already_exists(path: impl Display) -> ErrorData {
    let path = path.to_string();
    let mut details = ErrorDetails::new(ErrorCode::AlreadyExists, false);
    details.path = Some(path.clone());
    invalid_with_details(format!("target already exists: {path}"), details)
}

pub fn revision_conflict(expected: &str, actual: &str) -> ErrorData {
    let mut details = ErrorDetails::new(ErrorCode::RevisionConflict, false);
    details.expected_revision = Some(expected.to_owned());
    details.actual_revision = Some(actual.to_owned());
    invalid_with_details(
        format!("instance revision conflict; expected {expected}, actual {actual}"),
        details,
    )
}

pub fn permission_denied(operation: &str) -> ErrorData {
    invalid_with_details(
        format!("write access is disabled; cannot call {operation}"),
        ErrorDetails::new(ErrorCode::PermissionDenied, false),
    )
}

pub fn conflict(message: impl Into<String>) -> ErrorData {
    invalid_with_details(message, ErrorDetails::new(ErrorCode::Conflict, false))
}

pub fn unavailable(message: impl Into<String>) -> ErrorData {
    internal_with_details(message, ErrorDetails::new(ErrorCode::Unavailable, false))
}

pub fn app_error(error: AppError) -> ErrorData {
    match error {
        AppError::Cancelled => invalid_with_details(
            "operation was cancelled",
            ErrorDetails::new(ErrorCode::Cancelled, true),
        ),
        AppError::TaskAlreadyRunning { existing } => {
            let mut details = ErrorDetails::new(ErrorCode::TaskAlreadyRunning, true);
            details.task_id = Some(existing.get());
            invalid_with_details(
                format!("task {existing} is already installing this repository version"),
                details,
            )
        }
        AppError::RepositoryBusy { path } => {
            let mut details = ErrorDetails::new(ErrorCode::RepositoryBusy, true);
            details.path = Some(path.display().to_string());
            invalid_with_details(
                format!(
                    "repository is being modified by another RMCL process (lock: `{}`)",
                    path.display()
                ),
                details,
            )
        }
        AppError::InvalidRequest(message) => invalid_input(message),
        AppError::VersionNotFound(version) => not_found(format!(
            "version `{version}` was not found in the version manifest"
        )),
        AppError::InstallationIncomplete {
            version,
            missing_count,
            first_missing,
        } => conflict(format!(
            "version `{version}` is incomplete and needs repair ({missing_count} required files missing; first: {first_missing})"
        )),
        AppError::ContentDependency { message } => conflict(message),
        error @ (AppError::JavaNotConfigured
        | AppError::RemoteContentNotConfigured
        | AppError::InstanceCreatorNotConfigured
        | AppError::ForgeInstallNotConfigured) => unavailable(error.to_string()),
        other => internal_error(other),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    fn details(error: &ErrorData) -> &Value {
        error.data.as_ref().unwrap()
    }

    #[test]
    fn errors_expose_stable_codes_and_revision_details() {
        let error = revision_conflict("sha256:old", "sha256:new");
        assert_eq!(details(&error)["code"], "revision_conflict");
        assert_eq!(details(&error)["retryable"], false);
        assert_eq!(details(&error)["expected_revision"], "sha256:old");
        assert_eq!(details(&error)["actual_revision"], "sha256:new");
    }

    #[test]
    fn active_tasks_expose_the_existing_task_id() {
        let error = app_error(AppError::TaskAlreadyRunning {
            existing: rmcl_app::TaskId::from_raw(42),
        });
        assert_eq!(details(&error)["code"], "task_already_running");
        assert_eq!(details(&error)["retryable"], true);
        assert_eq!(details(&error)["task_id"], 42);
    }
}
