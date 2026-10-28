//! MCP adapters for RMCL's headless application services.

mod access;
mod error;
mod schema;
mod server;
mod transport;

pub use access::AccessLevel;
pub use error::McpServerError;
pub use server::McpServer;
pub use transport::{HttpServerConfig, serve_http, serve_stdio};
