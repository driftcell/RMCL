//! MCP adapters for RMCL's headless application services.

mod error;
mod schema;
mod server;
mod transport;

pub use error::McpServerError;
pub use server::McpServer;
pub use transport::{serve_http, serve_stdio};
