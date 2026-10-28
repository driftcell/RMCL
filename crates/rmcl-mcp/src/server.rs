use std::{
    collections::BTreeMap,
    future::Future,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use rmcl_app::{
    AppContext, AppError, ConflictPolicy, ContentGraphInstallRequest, ContentInstallPreflight,
    ContentKind, ContentPreflightItem, ContentPreflightStatus, CreateInstanceRequest,
    GameLaunchRequest, InstalledVersion, LoaderKind, LoaderSelection, RemoteProject,
    RemoteSearchQuery, RemoteSort, RemoteVersion, RepositoryPath, TaskId, VanillaInstallRequest,
    VersionSource, delete_instance_content, discover_installed_versions, import_instance_content,
    migrate_instance_to_isolated, preflight_content_install, rename_instance, scan_installed_mods,
    set_instance_content_enabled, set_instance_java, set_instance_launch_settings,
};
use rmcl_auth::{MicrosoftAccount, OfflineAccount};
use rmcl_domain::ServerAddress;
use rmcl_launch::LaunchAccount;
use rmcl_store::StoredAccountKind;
use rmcp::{
    ErrorData as McpError, ServerHandler, handler::server::wrapper::Parameters,
    model::CallToolResult, schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    error::{internal_error, invalid_input},
    schema::{
        content_entry, instance_revision, instance_summary, java_runtime, log_page, task_snapshot,
    },
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LoaderParams {
    /// Loader family: fabric, quilt, forge, or neoforge.
    kind: String,
    /// Loader version to install.
    version: String,
    /// Optional installer version; defaults to `version`.
    installer_version: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CreateInstanceParams {
    /// New instance directory ID.
    instance_id: String,
    /// Base Minecraft version ID.
    base_version_id: String,
    /// Optional display name; defaults to the instance ID.
    name: Option<String>,
    /// Optional loader to install.
    loader: Option<LoaderParams>,
    /// Optional source instance whose content should be copied.
    copy_content_from: Option<String>,
    /// Optional idempotency key used to deduplicate retries in this process.
    idempotency_key: Option<String>,
    /// Return the create plan without writing files or starting a task.
    dry_run: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WindowParams {
    width: u32,
    height: u32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct UpdateInstanceParams {
    /// Stable instance directory ID.
    instance_id: String,
    /// New display name.
    name: Option<String>,
    /// Explicit Java path; an empty string restores automatic selection.
    java: Option<String>,
    /// Memory in MiB; zero restores the JVM default.
    max_memory_mb: Option<u32>,
    /// Initial window dimensions.
    window: Option<WindowParams>,
    /// Clear the initial window override.
    clear_window: Option<bool>,
    /// Set to `isolated` to migrate the instance to an isolated game directory.
    isolation: Option<String>,
    /// Expected revision returned by the previous read; rejects concurrent writes.
    expected_revision: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RepairInstanceParams {
    /// Stable instance directory ID.
    instance_id: String,
    /// Expected revision returned by the previous read; rejects concurrent writes.
    expected_revision: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LaunchInstanceParams {
    /// Stable instance directory ID.
    instance_id: String,
    /// Run the full ensure/repair pass before launching.
    repair: Option<bool>,
    /// Optional server address to join immediately.
    server: Option<String>,
    /// Additional JVM arguments.
    additional_jvm_arguments: Option<Vec<String>>,
    /// Additional game arguments.
    additional_game_arguments: Option<Vec<String>>,
    /// Launch in demo mode.
    demo: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ContentImportParams {
    /// Stable instance directory ID.
    instance_id: String,
    /// Content kind: mod, resourcepack, or shader.
    kind: String,
    /// Local file to import.
    source: String,
    /// Conflict policy: rename or reject.
    conflict: Option<String>,
    /// Expected revision returned by the previous read; rejects concurrent writes.
    expected_revision: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ContentSetEnabledParams {
    /// Stable instance directory ID.
    instance_id: String,
    /// Content kind: mod, resourcepack, or shader.
    kind: String,
    /// Path relative to the content kind directory.
    relative_path: String,
    /// New enabled state.
    enabled: bool,
    /// Expected revision returned by the previous read; rejects concurrent writes.
    expected_revision: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ContentDeleteParams {
    /// Stable instance directory ID.
    instance_id: String,
    /// Content kind: mod, resourcepack, or shader.
    kind: String,
    /// Path relative to the content kind directory.
    relative_path: String,
    /// Expected revision returned by the previous read; required for deletion.
    expected_revision: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ContentSearchParams {
    /// Content kind: mod, resourcepack, or shader.
    kind: String,
    /// Free-form Modrinth query.
    query: String,
    /// Optional Minecraft version filter.
    game_version: Option<String>,
    /// Optional loader filter.
    loader: Option<String>,
    /// Result offset for pagination.
    offset: Option<usize>,
    /// Maximum results, from 1 to 100.
    limit: Option<usize>,
    /// Sort: relevance, downloads, follows, newest, or updated.
    sort: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ContentVersionsParams {
    /// Modrinth project ID or slug.
    project_id: String,
    /// Optional Minecraft version filter.
    game_version: Option<String>,
    /// Optional loader filter.
    loader: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ContentInstallParams {
    /// Stable instance directory ID.
    instance_id: String,
    /// Content kind: mod, resourcepack, or shader.
    kind: String,
    /// Modrinth version ID to install with required dependencies.
    version_id: String,
    /// Conflict policy: rename or reject.
    conflict: Option<String>,
    /// Expected revision returned by the previous read; rejects concurrent writes.
    expected_revision: Option<String>,
    /// Optional idempotency key used to deduplicate retries in this process.
    idempotency_key: Option<String>,
    /// Resolve dependencies and return preflight without starting a task.
    dry_run: Option<bool>,
}

#[tool_router]
impl McpServer {
    pub fn new(context: Arc<AppContext>) -> Self {
        Self { context }
    }

    #[tool(description = "Describe RMCL capabilities, paths, and available MCP tools")]
    async fn capabilities(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::structured(json!({
            "name": "rmcl",
            "version": env!("CARGO_PKG_VERSION"),
            "access": "read_write",
            "transports": ["stdio", "streamable-http"],
            "tools": [
                "capabilities",
                "instances_list",
                "instances_get",
                "instances_create",
                "instances_update",
                "instances_repair",
                "instances_launch",
                "content_list",
                "content_import",
                "content_set_enabled",
                "content_delete",
                "content_search",
                "content_versions",
                "content_install",
                "java_list",
                "tasks_list",
                "tasks_get",
                "tasks_logs",
                "tasks_cancel",
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
            "items": instances.iter().map(instance_summary).collect::<Result<Vec<_>, _>>()?
        })))
    }

    #[tool(description = "Get one installed Minecraft instance by ID")]
    async fn instances_get(
        &self,
        Parameters(params): Parameters<InstanceIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let instance = self.instance(&params.instance_id).await?;
        Ok(CallToolResult::structured(instance_summary(&instance)?))
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

    #[tool(description = "Create a Vanilla or mod-loader instance and return its install task")]
    async fn instances_create(
        &self,
        Parameters(params): Parameters<CreateInstanceParams>,
    ) -> Result<CallToolResult, McpError> {
        let loader = params
            .loader
            .map(|loader| {
                let kind = parse_loader_kind(&loader.kind)?;
                let installer_version = loader
                    .installer_version
                    .unwrap_or_else(|| loader.version.clone());
                Ok(LoaderSelection {
                    kind,
                    version: loader.version,
                    installer_version,
                })
            })
            .transpose()?;
        let request = CreateInstanceRequest {
            minecraft_root: self.context.minecraft_root.clone(),
            instance_id: params.instance_id.clone(),
            name: params.name.unwrap_or_else(|| params.instance_id.clone()),
            base_version_id: params.base_version_id,
            platform: self.context.platform.clone(),
            loader,
            copy_content_from: params.copy_content_from,
        };
        if params.dry_run == Some(true) {
            return Ok(CallToolResult::structured(json!({
                "dry_run": true,
                "instance_id": request.instance_id,
                "name": request.name,
                "base_version_id": request.base_version_id,
                "loader": request.loader.as_ref().map(|loader| json!({
                    "kind": loader.kind.as_str(),
                    "version": loader.version,
                    "installer_version": loader.installer_version,
                })),
                "copy_content_from": request.copy_content_from,
                "platform": {
                    "os": request.platform.os.native_directory_name(),
                    "architecture": request.platform.arch.as_str(),
                }
            })));
        }
        let (task, replay) = self
            .idempotent_task("create", params.idempotency_key.as_deref(), || {
                self.context.tasks.start_create_instance(request)
            })
            .await?;
        Ok(task_started(task, replay))
    }

    #[tool(
        description = "Rename an instance and update Java, memory, window, and isolation settings"
    )]
    async fn instances_update(
        &self,
        Parameters(params): Parameters<UpdateInstanceParams>,
    ) -> Result<CallToolResult, McpError> {
        let instance = self.instance(&params.instance_id).await?;
        self.expect_revision(&instance, params.expected_revision.as_deref())?;
        let layout = instance.instance_layout(self.context.minecraft_root.clone());
        if let Some(name) = params.name {
            rename_instance(&layout, &name).map_err(internal_error)?;
        }
        if let Some(java) = params.java {
            let java = java.trim();
            let path = (!java.is_empty()).then(|| PathBuf::from(java));
            if let Some(path) = &path {
                self.context
                    .java
                    .probe(path.clone())
                    .await
                    .map_err(internal_error)?;
            }
            set_instance_java(&layout, path).map_err(internal_error)?;
        }

        let mut memory = instance.max_memory_mb;
        let mut width = instance.window_width;
        let mut height = instance.window_height;
        if let Some(value) = params.max_memory_mb {
            memory = (value != 0).then_some(value);
        }
        if let Some(ref window) = params.window {
            width = Some(window.width);
            height = Some(window.height);
        }
        if params.clear_window == Some(true) {
            width = None;
            height = None;
        }
        if params.max_memory_mb.is_some()
            || params.window.is_some()
            || params.clear_window == Some(true)
        {
            set_instance_launch_settings(&layout, memory, width, height).map_err(internal_error)?;
        }

        if let Some(isolation) = params.isolation {
            match isolation.as_str() {
                "isolated" => {
                    if !instance.is_isolated() {
                        migrate_instance_to_isolated(
                            &self.context.minecraft_root,
                            &params.instance_id,
                            true,
                        )
                        .await
                        .map_err(internal_error)?;
                    }
                }
                "shared" => {
                    return Err(invalid_input(
                        "changing an instance back to shared mode is not supported",
                    ));
                }
                _ => {
                    return Err(invalid_input(format!(
                        "unsupported isolation mode `{isolation}`"
                    )));
                }
            }
        }
        self.instances_get(Parameters(InstanceIdParams {
            instance_id: params.instance_id,
        }))
        .await
    }

    #[tool(description = "Check and repair an installed instance and return the repair task")]
    async fn instances_repair(
        &self,
        Parameters(params): Parameters<RepairInstanceParams>,
    ) -> Result<CallToolResult, McpError> {
        let instance = self.instance(&params.instance_id).await?;
        self.expect_revision(&instance, params.expected_revision.as_deref())?;
        let request = VanillaInstallRequest {
            minecraft_root: self.context.minecraft_root.clone(),
            version_id: instance.id.as_str().to_owned(),
            platform: self.context.platform.clone(),
            source: VersionSource::Existing,
        };
        let task = self
            .context
            .tasks
            .start_install(request)
            .await
            .map_err(internal_error)?;
        Ok(task_started(task, false))
    }

    #[tool(description = "Launch an installed instance and return the game task")]
    async fn instances_launch(
        &self,
        Parameters(params): Parameters<LaunchInstanceParams>,
    ) -> Result<CallToolResult, McpError> {
        let instance = self.instance(&params.instance_id).await?;
        let account = self.launch_account().await?;
        let server = params
            .server
            .as_deref()
            .map(|address| ServerAddress::parse(address).map_err(internal_error))
            .transpose()?;
        let request = GameLaunchRequest {
            install: VanillaInstallRequest {
                minecraft_root: self.context.minecraft_root.clone(),
                version_id: instance.id.as_str().to_owned(),
                platform: self.context.platform.clone(),
                source: VersionSource::Existing,
            },
            account,
            java_executable: instance.java.clone(),
            max_memory_mb: instance.max_memory_mb,
            additional_jvm_arguments: params.additional_jvm_arguments.unwrap_or_default(),
            additional_game_arguments: params.additional_game_arguments.unwrap_or_default(),
            minecraft_version: instance.minecraft_version.clone(),
            server,
            resolution: instance.window_width.zip(instance.window_height),
            demo: params.demo.unwrap_or(false),
        };
        let task = if params.repair.unwrap_or(false) {
            self.context.tasks.start_launch(request).await
        } else {
            self.context.tasks.start_launch_installed(request).await
        }
        .map_err(internal_error)?;
        Ok(task_started(task, false))
    }

    #[tool(description = "Cooperatively cancel a running RMCL task")]
    async fn tasks_cancel(
        &self,
        Parameters(params): Parameters<TaskIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let cancelled = self
            .context
            .tasks
            .cancel(TaskId::from_raw(params.task_id))
            .await;
        if !cancelled {
            return Err(invalid_input(format!(
                "task {} is unknown or already terminal",
                params.task_id
            )));
        }
        let snapshot = self
            .context
            .tasks
            .snapshot(TaskId::from_raw(params.task_id))
            .await
            .ok_or_else(|| invalid_input(format!("unknown task ID {}", params.task_id)))?;
        Ok(CallToolResult::structured(task_snapshot(&snapshot)))
    }

    #[tool(description = "Import a local mod, resource pack, or shader pack into an instance")]
    async fn content_import(
        &self,
        Parameters(params): Parameters<ContentImportParams>,
    ) -> Result<CallToolResult, McpError> {
        let instance = self.instance(&params.instance_id).await?;
        self.expect_revision(&instance, params.expected_revision.as_deref())?;
        let kind = parse_content_kind(&params.kind)?;
        let conflict = parse_conflict_policy(params.conflict.as_deref())?;
        let layout = instance.instance_layout(self.context.minecraft_root.clone());
        let content = import_instance_content(layout, kind, PathBuf::from(params.source), conflict)
            .await
            .map_err(internal_error)?;
        Ok(CallToolResult::structured(json!({
            "content": content_entry(&content),
            "revision": instance_revision(&instance)?,
        })))
    }

    #[tool(description = "Enable or disable a local mod or resource pack")]
    async fn content_set_enabled(
        &self,
        Parameters(params): Parameters<ContentSetEnabledParams>,
    ) -> Result<CallToolResult, McpError> {
        let instance = self.instance(&params.instance_id).await?;
        self.expect_revision(&instance, params.expected_revision.as_deref())?;
        let kind = parse_content_kind(&params.kind)?;
        let layout = instance.instance_layout(self.context.minecraft_root.clone());
        let relative = RepositoryPath::new(params.relative_path).map_err(internal_error)?;
        let content = set_instance_content_enabled(layout, kind, relative, params.enabled)
            .await
            .map_err(internal_error)?;
        Ok(CallToolResult::structured(json!({
            "content": content_entry(&content),
            "revision": instance_revision(&instance)?,
        })))
    }

    #[tool(description = "Delete a local content entry after checking its expected revision")]
    async fn content_delete(
        &self,
        Parameters(params): Parameters<ContentDeleteParams>,
    ) -> Result<CallToolResult, McpError> {
        let instance = self.instance(&params.instance_id).await?;
        self.expect_revision(&instance, Some(&params.expected_revision))?;
        let kind = parse_content_kind(&params.kind)?;
        let layout = instance.instance_layout(self.context.minecraft_root.clone());
        let relative = RepositoryPath::new(params.relative_path).map_err(internal_error)?;
        delete_instance_content(layout, kind, relative)
            .await
            .map_err(internal_error)?;
        Ok(CallToolResult::structured(json!({
            "deleted": true,
            "revision": instance_revision(&instance)?,
        })))
    }

    #[tool(description = "Search Modrinth for content to install")]
    async fn content_search(
        &self,
        Parameters(params): Parameters<ContentSearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let kind = parse_content_kind(&params.kind)?;
        let mut query = RemoteSearchQuery::new(kind);
        query.query = params.query;
        query.game_version = params.game_version;
        query.loader = params.loader;
        query.offset = params.offset.unwrap_or(0);
        query.limit = params.limit.unwrap_or(20).clamp(1, 100);
        query.sort = parse_remote_sort(params.sort.as_deref())?;
        let page = self
            .context
            .modrinth
            .search(&query, &tokio_util::sync::CancellationToken::new())
            .await
            .map_err(internal_error)?;
        Ok(CallToolResult::structured(json!({
            "total_hits": page.total_hits,
            "offset": page.offset,
            "limit": page.limit,
            "items": page.projects.iter().map(remote_project_summary).collect::<Vec<_>>(),
        })))
    }

    #[tool(description = "List installable Modrinth versions for a project")]
    async fn content_versions(
        &self,
        Parameters(params): Parameters<ContentVersionsParams>,
    ) -> Result<CallToolResult, McpError> {
        let versions = self
            .context
            .modrinth
            .project_versions(
                &params.project_id,
                params.game_version.as_deref(),
                params.loader.as_deref(),
                &tokio_util::sync::CancellationToken::new(),
            )
            .await
            .map_err(internal_error)?;
        Ok(CallToolResult::structured(json!({
            "items": versions.iter().map(remote_version_summary).collect::<Vec<_>>()
        })))
    }

    #[tool(
        description = "Install a Modrinth version with dependencies, or return preflight when dry_run is true"
    )]
    async fn content_install(
        &self,
        Parameters(params): Parameters<ContentInstallParams>,
    ) -> Result<CallToolResult, McpError> {
        let instance = self.instance(&params.instance_id).await?;
        self.expect_revision(&instance, params.expected_revision.as_deref())?;
        let kind = parse_content_kind(&params.kind)?;
        let version = self
            .context
            .modrinth
            .version(
                &params.version_id,
                &tokio_util::sync::CancellationToken::new(),
            )
            .await
            .map_err(internal_error)?;
        let mut request = ContentGraphInstallRequest {
            instance: instance.instance_layout(self.context.minecraft_root.clone()),
            kind,
            version: version.clone(),
            display_name: format!("{} {}", version.project_id, version.version_number),
            conflict: parse_conflict_policy(params.conflict.as_deref())?,
            game_version: instance.minecraft_version.clone(),
            loader: instance
                .loader
                .as_ref()
                .map(|loader| loader.kind.as_str().to_owned()),
            skip_version_ids: Vec::new(),
        };
        let installed = tokio::task::spawn_blocking({
            let layout = request.instance.clone();
            move || scan_installed_mods(&layout)
        })
        .await
        .map_err(internal_error)?;
        let preflight = preflight_content_install(
            &self.context.modrinth,
            &request,
            &installed,
            Some(&self.context.mod_metadata),
            &tokio_util::sync::CancellationToken::new(),
        )
        .await
        .map_err(internal_error)?;
        if params.dry_run == Some(true) {
            return Ok(CallToolResult::structured(preflight_json(&preflight)));
        }
        if !preflight.incompatible.is_empty() {
            let conflicts = preflight
                .incompatible
                .iter()
                .map(|item| item.title.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(invalid_input(format!(
                "content install blocked by incompatible installed mods: {conflicts}"
            )));
        }
        request.skip_version_ids = preflight.skip_version_ids;
        let (task, replay) = self
            .idempotent_task("content_install", params.idempotency_key.as_deref(), || {
                self.context
                    .tasks
                    .start_content_install_with_dependencies(request)
            })
            .await?;
        Ok(task_started(task, replay))
    }

    async fn launch_account(&self) -> Result<LaunchAccount, McpError> {
        let config = self.context.config_store.load().map_err(internal_error)?;
        let selected = config
            .selected_account
            .as_deref()
            .ok_or_else(|| invalid_input("no account is selected"))?;
        match config.selected_account_kind {
            StoredAccountKind::Offline => {
                let stored = config
                    .accounts
                    .iter()
                    .find(|account| account.username == selected)
                    .ok_or_else(|| {
                        invalid_input("the selected offline account no longer exists")
                    })?;
                let account = match Uuid::parse_str(&stored.uuid) {
                    Ok(uuid) => {
                        OfflineAccount::with_uuid(&stored.username, uuid).map_err(internal_error)?
                    }
                    Err(_) => OfflineAccount::new(&stored.username).map_err(internal_error)?,
                };
                Ok(LaunchAccount::from_offline(&account))
            }
            StoredAccountKind::Microsoft => {
                let stored = config
                    .microsoft_accounts
                    .iter()
                    .find(|account| account.username == selected)
                    .ok_or_else(|| {
                        invalid_input("the selected Microsoft account no longer exists")
                    })?;
                let uuid = Uuid::parse_str(&stored.uuid).map_err(internal_error)?;
                let profile =
                    MicrosoftAccount::new(&stored.username, uuid).map_err(internal_error)?;
                let resolved = self
                    .context
                    .microsoft_accounts
                    .resolve_launch_account(&profile, &tokio_util::sync::CancellationToken::new())
                    .await
                    .map_err(internal_error)?;
                Ok(resolved.launch)
            }
        }
    }

    fn expect_revision(
        &self,
        instance: &InstalledVersion,
        expected: Option<&str>,
    ) -> Result<(), McpError> {
        let Some(expected) = expected else {
            return Ok(());
        };
        if expected.trim().is_empty() {
            return Err(invalid_input("expected_revision cannot be blank"));
        }
        let actual = instance_revision(instance)?;
        if actual != expected {
            return Err(invalid_input(format!(
                "instance revision conflict; expected {expected}, actual {actual}"
            )));
        }
        Ok(())
    }

    async fn idempotent_task<F>(
        &self,
        operation: &str,
        key: Option<&str>,
        start: impl FnOnce() -> F,
    ) -> Result<(TaskId, bool), McpError>
    where
        F: Future<Output = Result<TaskId, AppError>>,
    {
        let Some(key) = key else {
            return Ok((start().await.map_err(internal_error)?, false));
        };
        if key.trim().is_empty() {
            return Err(invalid_input("idempotency_key cannot be blank"));
        }
        let key = format!(
            "{operation}:{}:{key}",
            self.context.config_store.path().display()
        );
        let tasks = IDEMPOTENCY_TASKS.get_or_init(|| Mutex::new(BTreeMap::new()));
        let mut tasks = tasks.lock().await;
        if let Some(existing) = tasks.get(&key).copied() {
            return Ok((existing, true));
        }
        // Keep the registry locked while registering the task so concurrent
        // retries cannot both pass the initial lookup and start duplicate work.
        let task = start().await.map_err(internal_error)?;
        tasks.insert(key, task);
        Ok((task, false))
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
    instructions = "Read and write RMCL instances, content, Java runtimes, and task lifecycles"
)]
impl ServerHandler for McpServer {}

static IDEMPOTENCY_TASKS: OnceLock<Mutex<BTreeMap<String, TaskId>>> = OnceLock::new();

fn task_started(task: TaskId, replay: bool) -> CallToolResult {
    CallToolResult::structured(json!({
        "task_id": task.get(),
        "status": "running",
        "idempotent_replay": replay,
    }))
}

fn remote_project_summary(project: &RemoteProject) -> serde_json::Value {
    json!({
        "id": project.id,
        "slug": project.slug,
        "title": project.title,
        "description": project.description,
        "author": project.author,
        "downloads": project.downloads,
        "follows": project.follows,
        "categories": project.categories,
        "game_versions": project.game_versions,
        "loaders": project.loaders,
        "project_type": project.project_type,
        "updated": project.updated,
        "page_url": project.page_url(),
    })
}

fn remote_version_summary(version: &RemoteVersion) -> serde_json::Value {
    json!({
        "id": version.id,
        "project_id": version.project_id,
        "name": version.name,
        "version_number": version.version_number,
        "version_type": version.version_type,
        "published": version.published,
        "game_versions": version.game_versions,
        "loaders": version.loaders,
        "files": version.files.iter().map(|file| json!({
            "filename": file.filename,
            "url": file.url.to_string(),
            "size": file.size,
            "sha1": file.sha1.map(|sha1| sha1.to_string()),
            "primary": file.primary,
        })).collect::<Vec<_>>(),
        "dependencies": version.dependencies.iter().map(|dependency| json!({
            "version_id": dependency.version_id,
            "project_id": dependency.project_id,
            "file_name": dependency.file_name,
            "kind": format!("{:?}", dependency.kind).to_lowercase(),
        })).collect::<Vec<_>>(),
    })
}

fn preflight_json(preflight: &ContentInstallPreflight) -> serde_json::Value {
    json!({
        "dry_run": true,
        "main": preflight_item_json(&preflight.main),
        "dependencies": preflight.dependencies.iter().map(preflight_item_json).collect::<Vec<_>>(),
        "incompatible": preflight.incompatible.iter().map(preflight_item_json).collect::<Vec<_>>(),
        "optional_count": preflight.optional_count,
        "warnings": preflight.warnings.clone(),
        "skip_version_ids": preflight.skip_version_ids,
    })
}

fn preflight_item_json(item: &ContentPreflightItem) -> serde_json::Value {
    let status = match &item.status {
        ContentPreflightStatus::AlreadyInstalled { enabled } => json!({
            "state": "already_installed",
            "enabled": enabled,
        }),
        ContentPreflightStatus::InstalledDifferentVersion { version, enabled } => json!({
            "state": "installed_different_version",
            "version": version,
            "enabled": enabled,
        }),
        ContentPreflightStatus::WillInstall => json!({ "state": "will_install" }),
        ContentPreflightStatus::Conflict => json!({ "state": "conflict" }),
    };
    json!({
        "project_id": item.project_id,
        "project_slug": item.project_slug,
        "title": item.title,
        "version_id": item.version_id,
        "version_number": item.version_number,
        "file_name": item.file_name,
        "status": status,
    })
}

fn parse_conflict_policy(value: Option<&str>) -> Result<ConflictPolicy, McpError> {
    match value.unwrap_or("rename") {
        "rename" => Ok(ConflictPolicy::Rename),
        "reject" => Ok(ConflictPolicy::Reject),
        other => Err(invalid_input(format!(
            "unsupported conflict policy `{other}`; expected rename or reject"
        ))),
    }
}

fn parse_remote_sort(value: Option<&str>) -> Result<RemoteSort, McpError> {
    match value.unwrap_or("relevance") {
        "relevance" => Ok(RemoteSort::Relevance),
        "downloads" => Ok(RemoteSort::Downloads),
        "follows" => Ok(RemoteSort::Follows),
        "newest" => Ok(RemoteSort::Newest),
        "updated" => Ok(RemoteSort::Updated),
        other => Err(invalid_input(format!("unsupported sort `{other}`"))),
    }
}

fn parse_loader_kind(value: &str) -> Result<LoaderKind, McpError> {
    match value {
        "fabric" => Ok(LoaderKind::Fabric),
        "quilt" => Ok(LoaderKind::Quilt),
        "forge" => Ok(LoaderKind::Forge),
        "neoforge" => Ok(LoaderKind::NeoForge),
        _ => Err(invalid_input(format!("unsupported loader kind `{value}`"))),
    }
}

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

#[cfg(test)]
mod tests;
