use std::fmt::Display;

use rmcp::ErrorData;

#[derive(Debug, thiserror::Error)]
pub enum McpServerError {
    #[error("failed to bind MCP server: {0}")]
    Bind(#[from] std::io::Error),
    #[error("MCP server failed: {0}")]
    Serve(String),
}

pub fn internal_error(error: impl Display) -> ErrorData {
    ErrorData::internal_error(error.to_string(), None)
}

pub fn invalid_input(message: impl Into<String>) -> ErrorData {
    ErrorData::invalid_params(message.into(), None)
}
