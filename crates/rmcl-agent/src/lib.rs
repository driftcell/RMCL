//! Built-in AI assistant runtime for RMCL.

pub mod events;
mod mcp;
mod model;
mod provider;
mod runtime;

pub use mcp::{
    ASSISTANT_TOOL_ALLOWLIST, AgentMcp, StatusSink, ToolPolicy, connect as connect_mcp, wait,
};
pub use model::build_model;
pub use provider::{
    AGENT_KEYRING_SERVICE, AgentError, AgentKeyStore, AgentProvider, AgentSettings,
};
pub use runtime::{AgentService, AgentTurnContext};
