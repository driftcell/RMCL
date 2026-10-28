use std::sync::Arc;

use rmcl_app::{AppContext, ContentKind, InstalledVersion, TaskId, discover_installed_versions};
use rmcp::{
    ErrorData as McpError, ServerHandler, handler::server::wrapper::Parameters,
    model::CallToolResult, schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    error::{internal_error, invalid_input},
    schema::{content_entry, instance_summary, java_runtime, log_page, task_snapshot},
};

#[derive(Clone)]
pub struct McpServer {
    context: Arc<AppContext>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct InstanceIdParams {
    /// Stable instance directory ID.
    instance_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ContentListParams {
    /// Stable instance directory ID.
    instance_id: String,
    /// Optional content kind: mod, resourcepack, or shader.
    kind: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TaskIdParams {
    /// Task ID returned by an RMCL task operation.
    task_id: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TaskLogsParams {
    /// Task ID returned by an RMCL task operation.
    task_id: u64,
    /// Zero-based log line cursor returned by the previous page.
    cursor: Option<usize>,
    /// Maximum lines per page, from 1 to 1000; defaults to 200.
    limit: Option<usize>,
}

#[tool_router]
impl McpServer {
    pub fn new(context: Arc<AppContext>) -> Self {
        Self { context }
    }

    #[tool(description = "Describe RMCL capabilities, paths, and read-only MCP tools")]
    async fn capabilities(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::structured(json!({
            "name": "rmcl",
            "version": env!("CARGO_PKG_VERSION"),
            "read_only": true,
            "transports": ["stdio", "streamable-http"],
            "tools": [
                "capabilities",
                "instances_list",
                "instances_get",
                "content_list",
                "java_list",
                "tasks_list",
                "tasks_get",
                "tasks_logs",
            ],
            "platform": {
                "os": self.context.platform.os.native_directory_name(),
                "os_version": self.context.platform.os_version,
                "architecture": self.context.platform.arch.as_str(),
            },
            "paths": {
                "config_file": self.context.config_store.path().display().to_string(),
                "minecraft_root": self.context.minecraft_root.display().to_string(),
                "java_store": self.context.java_store.display().to_string(),
            }
        })))
    }

    #[tool(description = "List installed Minecraft instances and their configuration")]
    async fn instances_list(&self) -> Result<CallToolResult, McpError> {
        let instances = self.installed_versions().await?;
        Ok(CallToolResult::structured(json!({
            "items": instances.iter().map(instance_summary).collect::<Vec<_>>()
        })))
    }

    #[tool(description = "Get one installed Minecraft instance by ID")]
    async fn instances_get(
        &self,
        Parameters(params): Parameters<InstanceIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let instance = self.instance(&params.instance_id).await?;
        Ok(CallToolResult::structured(instance_summary(&instance)))
    }

    #[tool(description = "List local mods, resource packs, and shader packs of an instance")]
    async fn content_list(
        &self,
        Parameters(params): Parameters<ContentListParams>,
    ) -> Result<CallToolResult, McpError> {
        let instance = self.instance(&params.instance_id).await?;
        let kind = params.kind.as_deref().map(parse_content_kind).transpose()?;
        let layout = instance.instance_layout(self.context.minecraft_root.clone());
        let library = rmcl_app::scan_instance_content(layout)
            .await
            .map_err(internal_error)?;
        let items = library
            .items
            .iter()
            .filter(|item| kind.is_none_or(|kind| item.kind == kind))
            .map(content_entry)
            .collect::<Vec<_>>();
        Ok(CallToolResult::structured(json!({
            "instance_id": instance.id.as_str(),
            "items": items,
            "warnings": library.warnings.iter().map(|warning| warning.message()).collect::<Vec<_>>(),
        })))
    }

    #[tool(description = "Probe managed, JAVA_HOME, and PATH Java runtimes")]
    async fn java_list(&self) -> Result<CallToolResult, McpError> {
        let discovery = self.context.java.discover().await;
        Ok(CallToolResult::structured(json!({
            "items": discovery.runtimes.iter().map(java_runtime).collect::<Vec<_>>(),
            "failures": discovery.failures.iter().map(|failure| json!({
                "executable": failure.executable.display().to_string(),
                "error": failure.error.to_string(),
            })).collect::<Vec<_>>(),
        })))
    }

    #[tool(description = "List current and recently completed RMCL tasks")]
    async fn tasks_list(&self) -> Result<CallToolResult, McpError> {
        let snapshots = self.context.tasks.snapshots().await;
        Ok(CallToolResult::structured(json!({
            "items": snapshots.iter().map(task_snapshot).collect::<Vec<_>>()
        })))
    }

    #[tool(description = "Get one RMCL task by ID")]
    async fn tasks_get(
        &self,
        Parameters(params): Parameters<TaskIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let snapshot = self
            .context
            .tasks
            .snapshot(TaskId::from_raw(params.task_id))
            .await
            .ok_or_else(|| invalid_input(format!("unknown task ID {}", params.task_id)))?;
        Ok(CallToolResult::structured(task_snapshot(&snapshot)))
    }

    #[tool(description = "Read a bounded page of redacted logs for an RMCL task")]
    async fn tasks_logs(
        &self,
        Parameters(params): Parameters<TaskLogsParams>,
    ) -> Result<CallToolResult, McpError> {
        let logs = self
            .context
            .tasks
            .logs(TaskId::from_raw(params.task_id))
            .await
            .ok_or_else(|| invalid_input(format!("unknown task ID {}", params.task_id)))?;
        let page = log_page(&logs, params.cursor, params.limit)?;
        Ok(CallToolResult::structured(page))
    }

    async fn installed_versions(&self) -> Result<Vec<InstalledVersion>, McpError> {
        let root = self.context.minecraft_root.clone();
        tokio::task::spawn_blocking(move || discover_installed_versions(root))
            .await
            .map_err(internal_error)?
            .map_err(internal_error)
    }

    async fn instance(&self, id: &str) -> Result<InstalledVersion, McpError> {
        self.installed_versions()
            .await?
            .into_iter()
            .find(|instance| instance.id.as_str() == id)
            .ok_or_else(|| invalid_input(format!("unknown instance ID `{id}`")))
    }
}

#[tool_handler(
    name = "rmcl",
    instructions = "Read-only access to RMCL instances, content, Java runtimes, and task logs"
)]
impl ServerHandler for McpServer {}

fn parse_content_kind(value: &str) -> Result<ContentKind, McpError> {
    match value {
        "mod" => Ok(ContentKind::Mod),
        "resourcepack" => Ok(ContentKind::ResourcePack),
        "shader" => Ok(ContentKind::ShaderPack),
        _ => Err(invalid_input(format!(
            "unsupported content kind `{value}`; expected mod, resourcepack, or shader"
        ))),
    }
}
