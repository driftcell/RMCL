//! In-process MCP connections for the built-in assistant.

use std::{collections::BTreeMap, sync::Arc};

use rig::tool::rmcp::{McpConnection, McpTool, rmcp};
use rmcl_app::AppContext;
use rmcl_mcp::{AccessLevel, McpServer};
use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ToolAnnotations},
    schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::json;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::AgentError;

/// The tools the built-in assistant may use in the first version.
pub const ASSISTANT_TOOL_ALLOWLIST: &[&str] = &[
    "capabilities",
    "instances_list",
    "instances_get",
    "instances_create",
    "instances_update",
    "instances_repair",
    "instances_launch",
    "instances_delete",
    "versions_list",
    "loaders_list",
    "content_list",
    "content_search",
    "content_versions",
    "content_install",
    "content_update",
    "content_set_enabled",
    "content_delete",
    "java_list",
    "java_ensure",
    "accounts_list",
    "servers_list",
    "servers_ping",
    "skins_list",
    "skins_profile",
    "tasks_list",
    "tasks_get",
    "tasks_wait",
    "tasks_logs",
    "tasks_cancel",
];

/// Permission metadata copied from an MCP tool definition.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToolPolicy {
    pub read_only: bool,
    pub destructive: bool,
}

impl ToolPolicy {
    fn from_annotations(annotations: Option<&ToolAnnotations>) -> Self {
        Self {
            read_only: annotations
                .and_then(|annotations| annotations.read_only_hint)
                .unwrap_or(false),
            destructive: annotations
                .and_then(|annotations| annotations.destructive_hint)
                .unwrap_or(false),
        }
    }

    /// Tools that always need explicit user confirmation, even if their MCP
    /// annotations are not destructive.
    pub fn requires_confirmation(self, name: &str) -> bool {
        self.destructive
            || matches!(
                name,
                "instances_launch"
                    | "content_update"
                    | "tasks_cancel"
                    | "accounts_select"
                    | "accounts_create_offline"
                    | "accounts_delete"
                    | "skins_apply"
                    | "skins_reset"
                    | "skins_import"
                    | "skins_rename"
                    | "skins_delete"
                    | "servers_upsert"
                    | "servers_delete"
            )
    }
}

pub struct AgentMcp {
    pub connection: McpConnection,
    pub tools: Vec<McpTool>,
    pub policies: BTreeMap<String, ToolPolicy>,
    pub status_sink: StatusSink,
    #[allow(dead_code)]
    server_task: JoinHandle<()>,
    #[allow(dead_code)]
    status_task: JoinHandle<()>,
}

/// Connects to the existing MCP server in-process, sharing `AppContext` and
/// `TaskManager`, and filters the tools exposed to the model.
pub async fn connect(context: Arc<AppContext>) -> Result<AgentMcp, AgentError> {
    let (connection, server_task) = connect_server(
        McpServer::with_access(context.clone(), AccessLevel::ReadWrite),
        "rmcl",
    )
    .await?;
    let all_tools = connection
        .tools()
        .await
        .map_err(|error| AgentError::Mcp(error.to_string()))?;
    let mut tools = Vec::new();
    let mut policies = BTreeMap::new();
    for tool in all_tools {
        let name = tool.name().to_owned();
        if !ASSISTANT_TOOL_ALLOWLIST.contains(&name.as_str()) {
            continue;
        }
        policies.insert(
            name,
            ToolPolicy::from_annotations(tool.definition().annotations.as_ref()),
        );
        tools.push(tool);
    }

    let status_sink = StatusSink::default();
    let (status_connection, status_task) =
        connect_server(StatusServer::new(status_sink.clone()), "rmcl-agent-status").await?;
    let status_tools = status_connection
        .tools()
        .await
        .map_err(|error| AgentError::Mcp(error.to_string()))?;
    for tool in status_tools {
        policies.insert(tool.name().to_owned(), ToolPolicy::default());
        tools.push(tool);
    }

    drop(status_connection);
    Ok(AgentMcp {
        connection,
        tools,
        policies,
        status_sink,
        server_task,
        status_task,
    })
}

async fn connect_server<S>(
    server: S,
    _name: &str,
) -> Result<(McpConnection, JoinHandle<()>), AgentError>
where
    S: rmcp::handler::server::ServerHandler,
{
    let (client_io, server_io) = tokio::io::duplex(256 * 1024);
    let (client_read, client_write) = tokio::io::split(client_io);
    let (server_read, server_write) = tokio::io::split(server_io);
    let server_task = tokio::spawn(async move {
        if let Ok(running) = server.serve((server_read, server_write)).await {
            let _ = running.waiting().await;
        }
    });
    let client = rmcp::model::ClientConfig::default()
        .serve((client_read, client_write))
        .await
        .map_err(|error| AgentError::Mcp(error.to_string()))?;
    Ok((McpConnection::new(client), server_task))
}

type StatusCallback = Arc<dyn Fn(String, Option<String>) + Send + Sync>;

/// Sink used by the internal status MCP tool.
#[derive(Clone, Default)]
pub struct StatusSink {
    inner: Arc<std::sync::Mutex<Option<StatusCallback>>>,
}

impl StatusSink {
    pub fn set(&self, callback: Arc<dyn Fn(String, Option<String>) + Send + Sync>) {
        if let Ok(mut inner) = self.inner.lock() {
            *inner = Some(callback);
        }
    }

    fn report(&self, message: String, detail: Option<String>) {
        let callback = self.inner.lock().ok().and_then(|inner| inner.clone());
        if let Some(callback) = callback {
            callback(message, detail);
        }
    }
}

#[derive(Clone)]
struct StatusServer {
    sink: StatusSink,
}

impl StatusServer {
    fn new(sink: StatusSink) -> Self {
        Self { sink }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StatusParams {
    /// What the assistant is doing right now, in the user's language.
    message: String,
    /// Optional extra detail.
    detail: Option<String>,
}

#[tool_router]
impl StatusServer {
    #[tool(
        description = "Report the current assistant activity to the user. This tool has no side effects.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    async fn agent_report_status(
        &self,
        Parameters(params): Parameters<StatusParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.sink.report(params.message, params.detail);
        Ok(CallToolResult::structured(json!({"ok": true})))
    }
}

#[tool_handler(
    name = "rmcl-agent-status",
    instructions = "Internal assistant status reporting"
)]
impl ServerHandler for StatusServer {}

/// Waits for the in-process MCP server to end.
pub async fn wait(connection: &McpConnection, cancellation: CancellationToken) {
    tokio::select! {
        _ = cancellation.cancelled() => {}
        _ = connection.closed() => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destructive_and_special_tools_require_confirmation() {
        let destructive = ToolPolicy {
            read_only: false,
            destructive: true,
        };
        assert!(destructive.requires_confirmation("instances_delete"));
        assert!(ToolPolicy::default().requires_confirmation("instances_launch"));
        assert!(ToolPolicy::default().requires_confirmation("content_update"));
        assert!(!ToolPolicy::default().requires_confirmation("instances_create"));
        assert!(!ToolPolicy::default().requires_confirmation("content_search"));
    }

    #[test]
    fn assistant_tool_filter_excludes_notification_and_local_import_tools() {
        assert!(ASSISTANT_TOOL_ALLOWLIST.contains(&"accounts_list"));
        assert!(ASSISTANT_TOOL_ALLOWLIST.contains(&"tasks_wait"));
        assert!(!ASSISTANT_TOOL_ALLOWLIST.contains(&"tasks_subscribe"));
        assert!(!ASSISTANT_TOOL_ALLOWLIST.contains(&"content_import"));
        assert!(!ASSISTANT_TOOL_ALLOWLIST.contains(&"skins_apply"));
    }

    #[tokio::test]
    async fn in_process_connection_exposes_only_assistant_tools() {
        use rmcl_app::AppContext;
        use rmcl_store::{ConfigStore, LauncherConfig};

        let temporary = tempfile::TempDir::new().unwrap();
        let config = LauncherConfig {
            minecraft_root: temporary.path().join("minecraft"),
            java_store: temporary.path().join("java"),
            auto_install_java: false,
            ..LauncherConfig::default()
        };
        let context = AppContext::new(
            &config,
            ConfigStore::new(temporary.path().join("config.toml")),
        )
        .unwrap();
        let mcp = connect(context).await.unwrap();
        let names = mcp
            .tools
            .iter()
            .map(|tool| tool.name().to_owned())
            .collect::<Vec<_>>();
        assert!(names.iter().any(|name| name == "instances_list"));
        assert!(names.iter().any(|name| name == "tasks_wait"));
        assert!(names.iter().any(|name| name == "agent_report_status"));
        assert!(!names.iter().any(|name| name == "content_import"));
        assert!(!names.iter().any(|name| name == "tasks_subscribe"));
    }
}
