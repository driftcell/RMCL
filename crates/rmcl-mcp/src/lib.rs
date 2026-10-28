//! MCP adapters for RMCL's headless application services.

mod access;
mod error;
mod notifications;
mod schema;
mod server;
mod service;
mod transport;

pub use access::AccessLevel;
pub use error::McpServerError;
pub use server::McpServer;
pub use service::{DEFAULT_MCP_PORT, McpHttpService, McpHttpStatus};
pub use transport::{HttpServerConfig, HttpServerHandle, serve_http, serve_stdio};
