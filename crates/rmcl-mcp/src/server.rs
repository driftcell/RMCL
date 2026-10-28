use std::{
    collections::BTreeMap,
    future::Future,
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::Duration,
};

use rmcl_app::{
    AppContext, ConflictPolicy, ContentGraphInstallRequest, ContentInstallPreflight, ContentKind,
    ContentPreflightStatus, CreateInstanceRequest, GameLaunchRequest, InstalledVersion,
    JavaEnsureRequest, LoaderKind, LoaderSelection, MicrosoftAccountServiceError, PreparedSkin,
    RemoteSearchQuery, RemoteSort, RepositoryPath, RepositoryWriteGuard, ServerBookmarkError,
    SkinLibraryEntry, SkinLibraryError, SkinSourceError, TaskEvent, TaskId, TaskPhase, TaskStatus,
    VanillaInstallRequest, VersionSource, delete_installed_version_with_guard,
    delete_instance_content_with_guard, discover_installed_versions,
    import_instance_content_with_guard, load_skin_file, migrate_instance_to_isolated_with_guard,
    preflight_content_install, remove_server_bookmark, rename_instance_with_guard,
    scan_installed_mods, scan_instance_content, set_instance_content_enabled_with_guard,
    set_instance_java_with_guard, set_instance_launch_settings_with_guard, try_lock_repository,
    upsert_server_bookmark,
};
use rmcl_auth::{MicrosoftAccount, OfflineAccount, SkinVariant};
use rmcl_domain::ServerAddress;
use rmcl_instance::VersionId;
use rmcl_launch::LaunchAccount;
use rmcl_store::{LauncherConfig, StoredAccountKind};
use rmcp::{
    ErrorData as McpError, Peer, RoleServer, ServerHandler,
    handler::server::{tool::schema_for_type, wrapper::Parameters},
    model::CallToolResult,
    schemars, tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    access::AccessLevel,
    error::{
        already_exists, app_error, conflict, internal_error, invalid_input, not_found,
        permission_denied, revision_conflict,
    },
    notifications::TaskSubscriptions,
    schema::{
        AccountDelete, AccountKind, AccountList, AccountSummary, AccountWrite, Capabilities,
        ContentDelete, ContentInstallResult, ContentList, ContentUpdatePreflight,
        ContentUpdateResult, ContentWrite, CreateInstancePlan, CreateInstanceResult,
        CreateLoaderPlan, CreatePlatformPlan, InstanceDelete, InstanceDeletePlan,
        InstanceDeleteResult, InstanceList, InstanceSummary, JavaFailure, JavaList,
        LoaderVersionInfo, LoaderVersionList, LogPage, MicrosoftProfile, MinecraftVersionList,
        MinecraftVersionSummary, PathSummary, PlatformSummary, PreflightCheck,
        PreflightCheckStatus, ProfileCape, ProfileSkin, RemoteProjectSummary, RemoteSearchPage,
        RemoteVersionList, RemoteVersionSummary, ServerBookmark, ServerDelete, ServerList,
        ServerPingResult, ServerWrite, SkinApplyResult, SkinDelete, SkinEntry, SkinList, SkinWrite,
        TaskList, TaskSnapshot, TaskStarted, TaskSubscription, TaskSubscriptionEnded,
        TaskWaitResult, content_entry, game_log, instance_revision, instance_summary, java_runtime,
        log_page, task_snapshot,
    },
};

#[derive(Clone)]
pub struct McpServer {
    context: Arc<AppContext>,
    access: AccessLevel,
    subscriptions: Arc<TaskSubscriptions>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct InstanceIdParams {
    /// Stable instance directory ID.
    instance_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TaskWaitParams {
    /// Stable task ID returned by another tool.
    task_id: u64,
    /// Maximum seconds to wait; defaults to 120 and is capped at 600.
    timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AccountSelectorParams {
    /// Account username.
    username: String,
    /// Account kind: offline or microsoft.
    kind: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ServerUpsertParams {
    /// Existing bookmark ID to update; omit to create a bookmark.
    server_id: Option<String>,
    /// Display name; blank uses the normalized server address.
    name: String,
    /// Java Edition host, host:port, [IPv6], or [IPv6]:port.
    address: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ServerIdParams {
    /// Stable bookmark ID.
    server_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ServerPingParams {
    /// Bookmark ID to ping; exactly one of server_id or address is required.
    server_id: Option<String>,
    /// Address to ping without saving it; exactly one of server_id or address is required.
    address: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SkinImportParams {
    /// Local PNG path or direct HTTPS PNG URL.
    source: String,
    /// Optional display name; defaults to the normalized file name.
    name: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SkinRenameParams {
    /// Content SHA-256 ID of the library entry.
    skin_id: String,
    /// New display name.
    name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SkinIdParams {
    /// Content SHA-256 ID of the library entry.
    skin_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SkinProfileParams {
    /// Microsoft username; defaults to the selected Microsoft account.
    username: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SkinApplyParams {
    /// Existing local library content ID; exactly one of skin_id or source is required.
    skin_id: Option<String>,
    /// Local PNG path or direct HTTPS PNG URL; exactly one of skin_id or source is required.
    source: Option<String>,
    /// Arm model: classic or slim.
    variant: String,
    /// Microsoft username; defaults to the selected Microsoft account.
    username: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CreateOfflineAccountParams {
    /// Offline player name.
    username: String,
    /// Optional explicit UUID; defaults to the offline UUID derivation.
    uuid: Option<String>,
    /// Select this account after writing it.
    make_current: Option<bool>,
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
struct TaskSubscribeParams {
    /// Only receive events for this task; omit to subscribe to all tasks.
    task_id: Option<u64>,
    /// Also forward redacted game logs; defaults to false.
    include_logs: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TaskSubscriptionParams {
    /// Subscription ID returned by tasks_subscribe.
    subscription_id: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct JavaEnsureParams {
    /// Required Java major version; omit to accept any compatible runtime.
    major_version: Option<u32>,
    /// Optional Mojang runtime component, such as jre-legacy or java-runtime-alpha.
    component: Option<String>,
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

struct CreatePreflight {
    checks: Vec<PreflightCheck>,
    warnings: Vec<String>,
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
    /// Optional account used only for this launch; does not change the saved selection.
    account: Option<AccountSelectorParams>,
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
struct VersionListParams {
    /// Optional case-insensitive substring filter.
    query: Option<String>,
    /// Optional exact Minecraft version type, such as release or snapshot.
    version_type: Option<String>,
    /// Maximum results, from 1 to 200; defaults to 50.
    limit: Option<usize>,
    /// Refresh the remote manifest instead of using the cache.
    refresh: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LoaderVersionsParams {
    /// Loader family: fabric, quilt, forge, or neoforge.
    kind: String,
    /// Minecraft version used to select compatible loader versions.
    game_version: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DeleteInstanceParams {
    /// Stable instance directory ID.
    instance_id: String,
    /// Expected revision returned by the previous read; required for deletion.
    expected_revision: String,
    /// Return the deletion scope without removing files.
    dry_run: Option<bool>,
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

struct ContentInstallPreparation {
    request: ContentGraphInstallRequest,
    preflight: ContentInstallPreflight,
    repository_guard: RepositoryWriteGuard,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ContentUpdateParams {
    /// Stable instance directory ID.
    instance_id: String,
    /// Content kind: mod, resourcepack, or shader.
    kind: String,
    /// Existing relative path to replace while preserving enable state.
    relative_path: String,
    /// Modrinth version ID to install with required dependencies.
    version_id: String,
    /// Expected revision returned by the previous read; rejects concurrent writes.
    expected_revision: Option<String>,
    /// Optional idempotency key used to deduplicate retries in this process.
    idempotency_key: Option<String>,
    /// Resolve dependencies and return preflight without replacing the file.
    dry_run: Option<bool>,
}

struct ContentUpdatePreparation {
    request: ContentGraphInstallRequest,
    preflight: ContentInstallPreflight,
    relative_path: String,
    repository_guard: RepositoryWriteGuard,
}

#[tool_router]
impl McpServer {
    pub fn new(context: Arc<AppContext>) -> Self {
        Self::with_access(context, AccessLevel::ReadWrite)
    }

    pub fn with_access(context: Arc<AppContext>, access: AccessLevel) -> Self {
        Self {
            context,
            access,
            subscriptions: Arc::new(TaskSubscriptions::default()),
        }
    }

    fn require_write(&self, operation: &str) -> Result<(), McpError> {
        if self.access == AccessLevel::ReadWrite {
            Ok(())
        } else {
            Err(permission_denied(operation))
        }
    }

    #[tool(
        description = "Describe RMCL capabilities, paths, and available MCP tools",
        output_schema = schema_for_type::<Capabilities>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true)
    )]
    async fn capabilities(&self) -> Result<CallToolResult, McpError> {
        structured(Capabilities {
            name: "rmcl".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            access: self.access.as_str().to_owned(),
            transports: vec!["stdio".to_owned(), "streamable-http".to_owned()],
            tools: [
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
                "content_import",
                "content_set_enabled",
                "content_delete",
                "content_search",
                "content_versions",
                "content_install",
                "content_update",
                "java_list",
                "java_ensure",
                "accounts_list",
                "accounts_select",
                "accounts_create_offline",
                "accounts_delete",
                "servers_list",
                "servers_upsert",
                "servers_delete",
                "servers_ping",
                "skins_list",
                "skins_import",
                "skins_rename",
                "skins_delete",
                "skins_profile",
                "skins_apply",
                "skins_reset",
                "tasks_list",
                "tasks_get",
                "tasks_wait",
                "tasks_logs",
                "tasks_subscribe",
                "tasks_unsubscribe",
                "tasks_cancel",
            ]
            .map(str::to_owned)
            .to_vec(),
            platform: PlatformSummary {
                os: self.context.platform.os.native_directory_name().to_owned(),
                os_version: self.context.platform.os_version.clone(),
                architecture: self.context.platform.arch.as_str().to_owned(),
            },
            paths: PathSummary {
                config_file: self.context.config_store.path().display().to_string(),
                minecraft_root: self.context.minecraft_root.display().to_string(),
                java_store: self.context.java_store.display().to_string(),
            },
        })
    }

    #[tool(
        description = "List installed Minecraft instances and their configuration",
        output_schema = schema_for_type::<InstanceList>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true)
    )]
    async fn instances_list(&self) -> Result<CallToolResult, McpError> {
        let instances = self.installed_versions().await?;
        structured(InstanceList {
            items: instances
                .iter()
                .map(instance_summary)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }

    #[tool(
        description = "Get one installed Minecraft instance by ID",
        output_schema = schema_for_type::<InstanceSummary>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true)
    )]
    async fn instances_get(
        &self,
        Parameters(params): Parameters<InstanceIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let instance = self.instance(&params.instance_id).await?;
        structured(instance_summary(&instance)?)
    }

    #[tool(
        description = "List Minecraft versions from the official catalog with optional filtering",
        output_schema = schema_for_type::<MinecraftVersionList>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true)
    )]
    async fn versions_list(
        &self,
        Parameters(params): Parameters<VersionListParams>,
    ) -> Result<CallToolResult, McpError> {
        let manifest = self
            .context
            .catalog
            .manifest(
                params.refresh.unwrap_or(false),
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .map_err(app_error)?;
        let query = params.query.map(|query| query.to_lowercase());
        let items = manifest
            .versions
            .iter()
            .filter(|version| {
                query
                    .as_ref()
                    .is_none_or(|query| version.id.to_lowercase().contains(query))
                    && params
                        .version_type
                        .as_ref()
                        .is_none_or(|kind| version.version_type.as_ref() == Some(kind))
            })
            .take(params.limit.unwrap_or(50).clamp(1, 200))
            .map(|version| MinecraftVersionSummary {
                id: version.id.clone(),
                version_type: version.version_type.clone(),
                url: version.url.clone(),
                sha1: version.sha1.clone(),
                time: version.time.clone(),
                release_time: version.release_time.clone(),
            })
            .collect();
        structured(MinecraftVersionList {
            latest_release: manifest
                .latest
                .as_ref()
                .and_then(|latest| latest.release.clone()),
            latest_snapshot: manifest
                .latest
                .as_ref()
                .and_then(|latest| latest.snapshot.clone()),
            items,
        })
    }

    #[tool(
        description = "List compatible loader versions for a Minecraft version",
        output_schema = schema_for_type::<LoaderVersionList>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true)
    )]
    async fn loaders_list(
        &self,
        Parameters(params): Parameters<LoaderVersionsParams>,
    ) -> Result<CallToolResult, McpError> {
        let kind = parse_loader_kind(&params.kind)?;
        let versions = self
            .context
            .loaders
            .versions(
                kind,
                &params.game_version,
                &tokio_util::sync::CancellationToken::new(),
            )
            .await
            .map_err(app_error)?;
        structured(LoaderVersionList {
            kind: kind.as_str().to_owned(),
            game_version: params.game_version,
            items: versions
                .iter()
                .map(|version| LoaderVersionInfo {
                    version: version.version.clone(),
                    installer_version: version.installer_version.clone(),
                    stable: version.stable,
                })
                .collect(),
        })
    }

    #[tool(
        description = "List local mods, resource packs, and shader packs of an instance",
        output_schema = schema_for_type::<ContentList>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true)
    )]
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
        structured(ContentList {
            instance_id: instance.id.as_str().to_owned(),
            items,
            warnings: library
                .warnings
                .iter()
                .map(|warning| warning.message())
                .collect(),
        })
    }

    #[tool(
        description = "Probe managed, JAVA_HOME, and PATH Java runtimes",
        output_schema = schema_for_type::<JavaList>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true)
    )]
    async fn java_list(&self) -> Result<CallToolResult, McpError> {
        let discovery = self.context.java.discover().await;
        structured(JavaList {
            items: discovery.runtimes.iter().map(java_runtime).collect(),
            failures: discovery
                .failures
                .iter()
                .map(|failure| JavaFailure {
                    executable: failure.executable.display().to_string(),
                    error: failure.error.to_string(),
                })
                .collect(),
        })
    }

    #[tool(
        description = "Ensure a compatible Java runtime and return its install task",
        output_schema = schema_for_type::<TaskStarted>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false)
    )]
    async fn java_ensure(
        &self,
        Parameters(params): Parameters<JavaEnsureParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_write("java_ensure")?;
        let requirement = rmcl_java::JavaRequirement {
            component: params.component,
            major_version: params.major_version,
            maximum_major: None,
            operating_system: self.context.platform.os,
            architecture: self.context.platform.arch.clone(),
        };
        let task = self
            .context
            .tasks
            .start_java_ensure(JavaEnsureRequest {
                root: self.context.java_store.clone(),
                requirement,
            })
            .await
            .map_err(app_error)?;
        structured(task_started(task, false))
    }

    #[tool(
        description = "List offline and Microsoft accounts without exposing credentials",
        output_schema = schema_for_type::<AccountList>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true)
    )]
    async fn accounts_list(&self) -> Result<CallToolResult, McpError> {
        let config = self.context.config_store.load().map_err(internal_error)?;
        structured(AccountList {
            items: account_summaries(&config),
            selected_account: config.selected_account.clone(),
            selected_account_kind: account_kind(config.selected_account_kind),
        })
    }

    #[tool(
        description = "Select an existing account for later launches",
        output_schema = schema_for_type::<AccountWrite>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true)
    )]
    async fn accounts_select(
        &self,
        Parameters(params): Parameters<AccountSelectorParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_write("accounts_select")?;
        let _repository_guard = self.lock_repository()?;
        let kind = parse_account_kind(&params.kind)?;
        let mut config = self.context.config_store.load().map_err(internal_error)?;
        if account_summary(&config, &params.username, kind).is_none() {
            return Err(not_found(format!(
                "account `{}` was not found",
                params.username
            )));
        }
        config.selected_account = Some(params.username.clone());
        config.selected_account_kind = kind;
        self.context
            .config_store
            .save(&config)
            .map_err(internal_error)?;
        let account = account_summary(&config, &params.username, kind)
            .ok_or_else(|| internal_error("selected account was not written"))?;
        structured(AccountWrite { account })
    }

    #[tool(
        description = "Create or update an offline account with an optional explicit UUID",
        output_schema = schema_for_type::<AccountWrite>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true)
    )]
    async fn accounts_create_offline(
        &self,
        Parameters(params): Parameters<CreateOfflineAccountParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_write("accounts_create_offline")?;
        let _repository_guard = self.lock_repository()?;
        let account = match params.uuid.as_deref() {
            Some(uuid) => {
                let uuid =
                    Uuid::parse_str(uuid).map_err(|error| invalid_input(error.to_string()))?;
                OfflineAccount::with_uuid(&params.username, uuid).map_err(internal_error)?
            }
            None => OfflineAccount::new(&params.username).map_err(internal_error)?,
        };
        let stored = rmcl_store::StoredOfflineAccount {
            username: account.username().to_owned(),
            uuid: account.uuid().to_string(),
        };
        let mut config = self.context.config_store.load().map_err(internal_error)?;
        config
            .accounts
            .retain(|existing| existing.username != stored.username);
        config.accounts.push(stored.clone());
        if params.make_current.unwrap_or(false) {
            config.selected_account = Some(stored.username.clone());
            config.selected_account_kind = StoredAccountKind::Offline;
        }
        self.context
            .config_store
            .save(&config)
            .map_err(internal_error)?;
        structured(AccountWrite {
            account: account_summary(&config, &stored.username, StoredAccountKind::Offline)
                .ok_or_else(|| internal_error("offline account was not written"))?,
        })
    }

    #[tool(
        description = "Delete an offline or Microsoft account and its selected state",
        output_schema = schema_for_type::<AccountDelete>(),
        annotations(read_only_hint = false, destructive_hint = true, idempotent_hint = false)
    )]
    async fn accounts_delete(
        &self,
        Parameters(params): Parameters<AccountSelectorParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_write("accounts_delete")?;
        let _repository_guard = self.lock_repository()?;
        let kind = parse_account_kind(&params.kind)?;
        let mut config = self.context.config_store.load().map_err(internal_error)?;
        if account_summary(&config, &params.username, kind).is_none() {
            return Err(not_found(format!(
                "account `{}` was not found",
                params.username
            )));
        }
        match kind {
            StoredAccountKind::Offline => {
                config
                    .accounts
                    .retain(|account| account.username != params.username);
            }
            StoredAccountKind::Microsoft => {
                let stored = config
                    .microsoft_accounts
                    .iter()
                    .find(|account| account.username == params.username)
                    .ok_or_else(|| not_found("Microsoft account was not found"))?;
                let uuid = Uuid::parse_str(&stored.uuid).map_err(internal_error)?;
                let profile =
                    MicrosoftAccount::new(&stored.username, uuid).map_err(internal_error)?;
                self.context
                    .microsoft_accounts
                    .remove_credentials(&profile)
                    .await
                    .map_err(internal_error)?;
                config
                    .microsoft_accounts
                    .retain(|account| account.username != params.username);
            }
        }
        if config.selected_account.as_deref() == Some(params.username.as_str())
            && config.selected_account_kind == kind
        {
            config.selected_account = None;
        }
        ensure_account_selection(&mut config);
        self.context
            .config_store
            .save(&config)
            .map_err(internal_error)?;
        structured(AccountDelete {
            deleted: true,
            username: params.username,
            kind: account_kind(kind),
        })
    }

    #[tool(
        description = "List launcher-owned Java Edition server bookmarks without network access",
        output_schema = schema_for_type::<ServerList>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true)
    )]
    async fn servers_list(&self) -> Result<CallToolResult, McpError> {
        let config = self.context.config_store.load().map_err(internal_error)?;
        structured(ServerList {
            items: config.servers.iter().map(ServerBookmark::from).collect(),
        })
    }

    #[tool(
        description = "Create or update a Java Edition server bookmark",
        output_schema = schema_for_type::<ServerWrite>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false)
    )]
    async fn servers_upsert(
        &self,
        Parameters(params): Parameters<ServerUpsertParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_write("servers_upsert")?;
        let _repository_guard = self.lock_repository()?;
        let created = params.server_id.is_none();
        let mut config = self.context.config_store.load().map_err(internal_error)?;
        let server =
            upsert_server_bookmark(&mut config, params.server_id, &params.name, &params.address)
                .map_err(server_bookmark_error)?;
        self.context
            .config_store
            .save(&config)
            .map_err(internal_error)?;
        structured(ServerWrite {
            server: ServerBookmark::from(&server),
            created,
        })
    }

    #[tool(
        description = "Delete a launcher-owned server bookmark without changing servers.dat",
        output_schema = schema_for_type::<ServerDelete>(),
        annotations(read_only_hint = false, destructive_hint = true, idempotent_hint = false)
    )]
    async fn servers_delete(
        &self,
        Parameters(params): Parameters<ServerIdParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_write("servers_delete")?;
        let _repository_guard = self.lock_repository()?;
        let mut config = self.context.config_store.load().map_err(internal_error)?;
        remove_server_bookmark(&mut config, &params.server_id).map_err(server_bookmark_error)?;
        self.context
            .config_store
            .save(&config)
            .map_err(internal_error)?;
        structured(ServerDelete {
            deleted: true,
            server_id: params.server_id,
        })
    }

    #[tool(
        description = "Ping one Java Edition server and return MOTD, players, latency, and cached icon",
        output_schema = schema_for_type::<ServerPingResult>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true)
    )]
    async fn servers_ping(
        &self,
        Parameters(params): Parameters<ServerPingParams>,
    ) -> Result<CallToolResult, McpError> {
        let (server_id, raw_address) = match (params.server_id, params.address) {
            (Some(server_id), None) => {
                let config = self.context.config_store.load().map_err(internal_error)?;
                let server = config
                    .servers
                    .iter()
                    .find(|server| server.id == server_id)
                    .ok_or_else(|| not_found(format!("server `{server_id}` was not found")))?;
                (Some(server_id), server.address.clone())
            }
            (None, Some(address)) => (None, address),
            _ => {
                return Err(invalid_input(
                    "exactly one of server_id or address is required",
                ));
            }
        };
        let address =
            ServerAddress::parse(&raw_address).map_err(|error| invalid_input(error.to_string()))?;
        let status = self
            .context
            .servers
            .ping(&address, &tokio_util::sync::CancellationToken::new())
            .await
            .map_err(internal_error)?;
        let icon_path = match status.icon_png.as_deref() {
            Some(png) => Some(
                self.context
                    .server_icons
                    .ensure(png)
                    .await
                    .map_err(internal_error)?
                    .display()
                    .to_string(),
            ),
            None => None,
        };
        structured(ServerPingResult {
            server_id,
            address: address.to_string(),
            motd: status.motd,
            online_players: status.online_players,
            max_players: status.max_players,
            version_name: status.version_name,
            protocol: status.protocol,
            latency_ms: u64::try_from(status.latency.as_millis()).unwrap_or(u64::MAX),
            icon_path,
        })
    }

    #[tool(
        description = "List validated skins in the local RMCL skin library",
        output_schema = schema_for_type::<SkinList>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true)
    )]
    async fn skins_list(&self) -> Result<CallToolResult, McpError> {
        let entries = self
            .context
            .skin_library
            .list()
            .await
            .map_err(skin_library_error)?;
        structured(SkinList {
            items: entries
                .iter()
                .map(|entry| self.skin_entry(entry))
                .collect::<Result<Vec<_>, _>>()?,
        })
    }

    #[tool(
        description = "Validate and import a local PNG or direct HTTPS PNG into the skin library",
        output_schema = schema_for_type::<SkinWrite>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false)
    )]
    async fn skins_import(
        &self,
        Parameters(params): Parameters<SkinImportParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_write("skins_import")?;
        let source = params.source.trim();
        if source.is_empty() {
            return Err(invalid_input("skin source cannot be blank"));
        }
        let prepared = self.prepare_skin_source(source).await?;
        let _library_guard =
            try_lock_repository(self.context.skin_library.root()).map_err(app_error)?;
        let mut entry = self
            .context
            .skin_library
            .import(&prepared)
            .await
            .map_err(skin_library_error)?;
        if let Some(name) = params.name.as_deref().map(str::trim)
            && !name.is_empty()
        {
            self.context
                .skin_library
                .rename(&entry.id, name)
                .await
                .map_err(skin_library_error)?;
            entry.name = name.to_owned();
        }
        structured(SkinWrite {
            skin: self.skin_entry(&entry)?,
        })
    }

    #[tool(
        description = "Rename a local skin library entry without changing its content ID",
        output_schema = schema_for_type::<SkinWrite>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true)
    )]
    async fn skins_rename(
        &self,
        Parameters(params): Parameters<SkinRenameParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_write("skins_rename")?;
        let _library_guard =
            try_lock_repository(self.context.skin_library.root()).map_err(app_error)?;
        self.context
            .skin_library
            .rename(&params.skin_id, &params.name)
            .await
            .map_err(skin_library_error)?;
        let entries = self
            .context
            .skin_library
            .list()
            .await
            .map_err(skin_library_error)?;
        let entry = entries
            .iter()
            .find(|entry| entry.id == params.skin_id)
            .ok_or_else(|| not_found(format!("skin `{}` was not found", params.skin_id)))?;
        structured(SkinWrite {
            skin: self.skin_entry(entry)?,
        })
    }

    #[tool(
        description = "Delete a local skin library entry and its full texture",
        output_schema = schema_for_type::<SkinDelete>(),
        annotations(read_only_hint = false, destructive_hint = true, idempotent_hint = false)
    )]
    async fn skins_delete(
        &self,
        Parameters(params): Parameters<SkinIdParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_write("skins_delete")?;
        let _library_guard =
            try_lock_repository(self.context.skin_library.root()).map_err(app_error)?;
        self.context
            .skin_library
            .remove(&params.skin_id)
            .await
            .map_err(skin_library_error)?;
        structured(SkinDelete {
            deleted: true,
            skin_id: params.skin_id,
        })
    }

    #[tool(
        description = "Refresh a Microsoft profile and list owned skins and capes without exposing tokens",
        output_schema = schema_for_type::<MicrosoftProfile>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true)
    )]
    async fn skins_profile(
        &self,
        Parameters(params): Parameters<SkinProfileParams>,
    ) -> Result<CallToolResult, McpError> {
        let account = self.microsoft_account(params.username.as_deref()).await?;
        let profile = self
            .context
            .microsoft_accounts
            .refresh_profile(&account, &tokio_util::sync::CancellationToken::new())
            .await
            .map_err(microsoft_account_error)?;
        structured(microsoft_profile(&profile))
    }

    #[tool(
        description = "Upload a validated local or remote PNG skin to a Microsoft account and cache it locally",
        output_schema = schema_for_type::<SkinApplyResult>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false)
    )]
    async fn skins_apply(
        &self,
        Parameters(params): Parameters<SkinApplyParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_write("skins_apply")?;
        let prepared = match (params.skin_id, params.source) {
            (Some(skin_id), None) => self
                .context
                .skin_library
                .load(&skin_id)
                .await
                .map_err(skin_library_error)?,
            (None, Some(source)) => self.prepare_skin_source(&source).await?,
            _ => {
                return Err(invalid_input(
                    "exactly one of skin_id or source is required",
                ));
            }
        };
        let variant = parse_skin_variant(&params.variant)?;
        let account = self.microsoft_account(params.username.as_deref()).await?;
        let profile = self
            .context
            .microsoft_accounts
            .upload_skin(
                &account,
                variant,
                prepared.file_name(),
                prepared.bytes(),
                &tokio_util::sync::CancellationToken::new(),
            )
            .await
            .map_err(microsoft_account_error)?;
        let _library_guard =
            try_lock_repository(self.context.skin_library.root()).map_err(app_error)?;
        let entry = self
            .context
            .skin_library
            .import(&prepared)
            .await
            .map_err(|error| {
                conflict(format!(
                    "skin was uploaded for {}, but local library import failed: {error}",
                    profile.username()
                ))
            })?;
        structured(SkinApplyResult {
            profile: microsoft_profile(&profile),
            skin: self.skin_entry(&entry)?,
        })
    }

    #[tool(
        description = "Reset a Microsoft account to its default skin and refresh the returned profile if needed",
        output_schema = schema_for_type::<MicrosoftProfile>(),
        annotations(read_only_hint = false, destructive_hint = true, idempotent_hint = true)
    )]
    async fn skins_reset(
        &self,
        Parameters(params): Parameters<SkinProfileParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_write("skins_reset")?;
        let account = self.microsoft_account(params.username.as_deref()).await?;
        let profile = self
            .context
            .microsoft_accounts
            .reset_skin(&account, &tokio_util::sync::CancellationToken::new())
            .await
            .map_err(microsoft_account_error)?;
        let profile = match profile {
            Some(profile) => profile,
            None => self
                .context
                .microsoft_accounts
                .refresh_profile(&account, &tokio_util::sync::CancellationToken::new())
                .await
                .map_err(microsoft_account_error)?,
        };
        structured(microsoft_profile(&profile))
    }

    #[tool(
        description = "List current and recently completed RMCL tasks",
        output_schema = schema_for_type::<TaskList>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true)
    )]
    async fn tasks_list(&self) -> Result<CallToolResult, McpError> {
        let snapshots = self.context.tasks.snapshots().await;
        structured(TaskList {
            items: snapshots.iter().map(task_snapshot).collect(),
        })
    }

    #[tool(
        description = "Get one RMCL task by ID",
        output_schema = schema_for_type::<TaskSnapshot>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true)
    )]
    async fn tasks_get(
        &self,
        Parameters(params): Parameters<TaskIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let snapshot = self
            .context
            .tasks
            .snapshot(TaskId::from_raw(params.task_id))
            .await
            .ok_or_else(|| not_found(format!("unknown task ID {}", params.task_id)))?;
        structured(task_snapshot(&snapshot))
    }

    #[tool(
        description = "Wait for a task to finish, or for a launched game to start running, returning its current state on timeout",
        output_schema = schema_for_type::<TaskWaitResult>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true)
    )]
    async fn tasks_wait(
        &self,
        Parameters(params): Parameters<TaskWaitParams>,
    ) -> Result<CallToolResult, McpError> {
        let task_id = TaskId::from_raw(params.task_id);
        let timeout = Duration::from_secs(params.timeout_seconds.unwrap_or(120).clamp(1, 600));
        let deadline = tokio::time::Instant::now() + timeout;
        let mut events = self.context.tasks.subscribe();
        let mut snapshot = self
            .context
            .tasks
            .snapshot(task_id)
            .await
            .ok_or_else(|| not_found(format!("unknown task ID {}", params.task_id)))?;
        loop {
            if task_wait_satisfied(&snapshot) {
                return task_wait_result(&self.context, snapshot, false).await;
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return task_wait_result(&self.context, snapshot, true).await;
            }
            match tokio::time::timeout(remaining, events.recv()).await {
                Ok(Ok(TaskEvent::Snapshot(next))) if next.id == task_id => snapshot = next,
                Ok(Ok(TaskEvent::Dismissed(id))) if id == task_id => {
                    return Err(not_found(format!("unknown task ID {}", params.task_id)));
                }
                Ok(Ok(_)) => {}
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {
                    snapshot =
                        self.context.tasks.snapshot(task_id).await.ok_or_else(|| {
                            not_found(format!("unknown task ID {}", params.task_id))
                        })?;
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                    return task_wait_result(&self.context, snapshot, true).await;
                }
                Err(_) => return task_wait_result(&self.context, snapshot, true).await,
            }
        }
    }

    #[tool(
        description = "Read a bounded page of redacted logs for an RMCL task",
        output_schema = schema_for_type::<LogPage>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true)
    )]
    async fn tasks_logs(
        &self,
        Parameters(params): Parameters<TaskLogsParams>,
    ) -> Result<CallToolResult, McpError> {
        let logs = self
            .context
            .tasks
            .logs(TaskId::from_raw(params.task_id))
            .await
            .ok_or_else(|| not_found(format!("unknown task ID {}", params.task_id)))?;
        structured(log_page(&logs, params.cursor, params.limit)?)
    }

    #[tool(
        description = "Subscribe this MCP session to bounded task snapshot and optional log notifications",
        output_schema = schema_for_type::<TaskSubscription>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = false)
    )]
    async fn tasks_subscribe(
        &self,
        Parameters(params): Parameters<TaskSubscribeParams>,
        peer: Peer<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        structured(
            self.subscriptions
                .subscribe(
                    &self.context.tasks,
                    peer,
                    params.task_id,
                    params.include_logs.unwrap_or(false),
                )
                .await?,
        )
    }

    #[tool(
        description = "Stop one task notification subscription owned by this MCP session",
        output_schema = schema_for_type::<TaskSubscriptionEnded>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true)
    )]
    async fn tasks_unsubscribe(
        &self,
        Parameters(params): Parameters<TaskSubscriptionParams>,
    ) -> Result<CallToolResult, McpError> {
        structured(
            self.subscriptions
                .unsubscribe(params.subscription_id)
                .await?,
        )
    }

    async fn prepare_create_instance(
        &self,
        request: &CreateInstanceRequest,
    ) -> Result<CreatePreflight, McpError> {
        let id = VersionId::new(&request.instance_id)
            .map_err(|error| invalid_input(error.to_string()))?;
        let version_dir = self
            .context
            .minecraft_root
            .join("versions")
            .join(id.as_str());
        match version_dir.try_exists() {
            Ok(true) => return Err(already_exists(version_dir.display())),
            Ok(false) => {}
            Err(error) => return Err(internal_error(error)),
        }
        let mut checks = vec![PreflightCheck {
            name: "instance_id".to_owned(),
            status: PreflightCheckStatus::Passed,
            message: format!("instance ID `{}` is valid and available", id.as_str()),
        }];
        let manifest = self
            .context
            .catalog
            .manifest(false, tokio_util::sync::CancellationToken::new())
            .await
            .map_err(app_error)?;
        if manifest.find(&request.base_version_id).is_none() {
            return Err(not_found(format!(
                "Minecraft version `{}` was not found in the catalog",
                request.base_version_id
            )));
        }
        checks.push(PreflightCheck {
            name: "base_version".to_owned(),
            status: PreflightCheckStatus::Passed,
            message: format!("Minecraft {} is available", request.base_version_id),
        });
        if let Some(source) = &request.copy_content_from {
            self.instance(source).await?;
            checks.push(PreflightCheck {
                name: "copy_content_from".to_owned(),
                status: PreflightCheckStatus::Passed,
                message: format!("content source instance `{source}` exists"),
            });
        }
        let mut warnings = Vec::new();
        if let Some(loader) = &request.loader {
            let versions = self
                .context
                .loaders
                .versions(
                    loader.kind,
                    &request.base_version_id,
                    &tokio_util::sync::CancellationToken::new(),
                )
                .await
                .map_err(app_error)?;
            let Some(selected) = versions.iter().find(|candidate| {
                candidate.version == loader.version
                    && candidate.installer_version == loader.installer_version
            }) else {
                return Err(not_found(format!(
                    "loader {} {} was not found for Minecraft {}",
                    loader.kind.as_str(),
                    loader.version,
                    request.base_version_id
                )));
            };
            checks.push(PreflightCheck {
                name: "loader".to_owned(),
                status: if selected.stable {
                    PreflightCheckStatus::Passed
                } else {
                    PreflightCheckStatus::Warning
                },
                message: format!(
                    "{} {} is available for Minecraft {}",
                    loader.kind.as_str(),
                    loader.version,
                    request.base_version_id
                ),
            });
            if !selected.stable {
                warnings.push(format!(
                    "{} {} is not marked as stable",
                    loader.kind.as_str(),
                    loader.version
                ));
            }
        }
        Ok(CreatePreflight { checks, warnings })
    }

    #[tool(
        description = "Create a Vanilla or mod-loader instance and return its install task",
        output_schema = schema_for_type::<CreateInstanceResult>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false)
    )]
    async fn instances_create(
        &self,
        Parameters(params): Parameters<CreateInstanceParams>,
    ) -> Result<CallToolResult, McpError> {
        if params.dry_run != Some(true) {
            self.require_write("instances_create")?;
        }
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
        let preflight = self.prepare_create_instance(&request).await?;
        if params.dry_run == Some(true) {
            return structured(CreateInstanceResult::Plan(Box::new(CreateInstancePlan {
                dry_run: true,
                instance_id: request.instance_id,
                name: request.name,
                base_version_id: request.base_version_id,
                loader: request.loader.as_ref().map(|loader| CreateLoaderPlan {
                    kind: loader.kind.as_str().to_owned(),
                    version: loader.version.clone(),
                    installer_version: loader.installer_version.clone(),
                }),
                copy_content_from: request.copy_content_from,
                platform: CreatePlatformPlan {
                    os: request.platform.os.native_directory_name().to_owned(),
                    architecture: request.platform.arch.as_str().to_owned(),
                },
                checks: preflight.checks,
                warnings: preflight.warnings,
            })));
        }
        let (task, replay) = self
            .idempotent_task("create", params.idempotency_key.as_deref(), || async move {
                self.context
                    .tasks
                    .start_create_instance(request)
                    .await
                    .map_err(app_error)
            })
            .await?;
        structured(CreateInstanceResult::Task(task_started(task, replay)))
    }

    #[tool(
        description = "Rename an instance and update Java, memory, window, and isolation settings",
        output_schema = schema_for_type::<InstanceSummary>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true)
    )]
    async fn instances_update(
        &self,
        Parameters(params): Parameters<UpdateInstanceParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_write("instances_update")?;
        let instance = self.instance(&params.instance_id).await?;
        let repository_guard = self.lock_repository()?;
        self.expect_revision(&instance, params.expected_revision.as_deref())?;
        let layout = instance.instance_layout(self.context.minecraft_root.clone());
        if let Some(name) = params.name {
            rename_instance_with_guard(&layout, &name, &repository_guard)
                .map_err(internal_error)?;
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
            set_instance_java_with_guard(&layout, path, &repository_guard)
                .map_err(internal_error)?;
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
            set_instance_launch_settings_with_guard(
                &layout,
                memory,
                width,
                height,
                &repository_guard,
            )
            .map_err(internal_error)?;
        }

        if let Some(isolation) = params.isolation {
            match isolation.as_str() {
                "isolated" => {
                    if !instance.is_isolated() {
                        migrate_instance_to_isolated_with_guard(
                            &self.context.minecraft_root,
                            &params.instance_id,
                            true,
                            &repository_guard,
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

    #[tool(
        description = "Delete an installed instance after checking its expected revision",
        output_schema = schema_for_type::<InstanceDeleteResult>(),
        annotations(read_only_hint = false, destructive_hint = true, idempotent_hint = false)
    )]
    async fn instances_delete(
        &self,
        Parameters(params): Parameters<DeleteInstanceParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_write("instances_delete")?;
        let instance = self.instance(&params.instance_id).await?;
        let repository_guard = self.lock_repository()?;
        self.expect_revision(&instance, Some(&params.expected_revision))?;
        if params.dry_run == Some(true) {
            return structured(InstanceDeleteResult::Plan(InstanceDeletePlan {
                dry_run: true,
                instance_id: instance.id.as_str().to_owned(),
                version_directory: instance.version_dir.display().to_string(),
                deletes_isolated_content: instance.is_isolated(),
                preserves_shared_files: true,
            }));
        }
        delete_installed_version_with_guard(
            &self.context.minecraft_root,
            &params.instance_id,
            &repository_guard,
        )
        .await
        .map_err(app_error)?;
        structured(InstanceDeleteResult::Deleted(InstanceDelete {
            deleted: true,
            instance_id: params.instance_id,
        }))
    }

    #[tool(
        description = "Check and repair an installed instance and return the repair task",
        output_schema = schema_for_type::<TaskStarted>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false)
    )]
    async fn instances_repair(
        &self,
        Parameters(params): Parameters<RepairInstanceParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_write("instances_repair")?;
        let instance = self.instance(&params.instance_id).await?;
        let repository_guard = self.lock_repository()?;
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
            .start_install_with_guard(request, repository_guard)
            .await
            .map_err(internal_error)?;
        structured(task_started(task, false))
    }

    #[tool(
        description = "Launch an installed instance and return the game task",
        output_schema = schema_for_type::<TaskStarted>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false)
    )]
    async fn instances_launch(
        &self,
        Parameters(params): Parameters<LaunchInstanceParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_write("instances_launch")?;
        let instance = self.instance(&params.instance_id).await?;
        let account = self.launch_account(params.account.as_ref()).await?;
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
        structured(task_started(task, false))
    }

    #[tool(
        description = "Cooperatively cancel a running RMCL task",
        output_schema = schema_for_type::<TaskSnapshot>(),
        annotations(read_only_hint = false, destructive_hint = true, idempotent_hint = false)
    )]
    async fn tasks_cancel(
        &self,
        Parameters(params): Parameters<TaskIdParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_write("tasks_cancel")?;
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
            .ok_or_else(|| not_found(format!("unknown task ID {}", params.task_id)))?;
        structured(task_snapshot(&snapshot))
    }

    #[tool(
        description = "Import a local mod, resource pack, or shader pack into an instance",
        output_schema = schema_for_type::<ContentWrite>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false)
    )]
    async fn content_import(
        &self,
        Parameters(params): Parameters<ContentImportParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_write("content_import")?;
        let instance = self.instance(&params.instance_id).await?;
        let repository_guard = self.lock_repository()?;
        self.expect_revision(&instance, params.expected_revision.as_deref())?;
        let kind = parse_content_kind(&params.kind)?;
        let conflict = parse_conflict_policy(params.conflict.as_deref())?;
        let layout = instance.instance_layout(self.context.minecraft_root.clone());
        let content = import_instance_content_with_guard(
            layout,
            kind,
            PathBuf::from(params.source),
            conflict,
            &repository_guard,
        )
        .await
        .map_err(internal_error)?;
        structured(ContentWrite {
            content: content_entry(&content),
            revision: instance_revision(&instance)?,
        })
    }

    #[tool(
        description = "Enable or disable a local mod or resource pack",
        output_schema = schema_for_type::<ContentWrite>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true)
    )]
    async fn content_set_enabled(
        &self,
        Parameters(params): Parameters<ContentSetEnabledParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_write("content_set_enabled")?;
        let instance = self.instance(&params.instance_id).await?;
        let repository_guard = self.lock_repository()?;
        self.expect_revision(&instance, params.expected_revision.as_deref())?;
        let kind = parse_content_kind(&params.kind)?;
        let layout = instance.instance_layout(self.context.minecraft_root.clone());
        let relative = RepositoryPath::new(params.relative_path).map_err(internal_error)?;
        let content = set_instance_content_enabled_with_guard(
            layout,
            kind,
            relative,
            params.enabled,
            &repository_guard,
        )
        .await
        .map_err(internal_error)?;
        structured(ContentWrite {
            content: content_entry(&content),
            revision: instance_revision(&instance)?,
        })
    }

    #[tool(
        description = "Delete a local content entry after checking its expected revision",
        output_schema = schema_for_type::<ContentDelete>(),
        annotations(read_only_hint = false, destructive_hint = true, idempotent_hint = false)
    )]
    async fn content_delete(
        &self,
        Parameters(params): Parameters<ContentDeleteParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_write("content_delete")?;
        let instance = self.instance(&params.instance_id).await?;
        let repository_guard = self.lock_repository()?;
        self.expect_revision(&instance, Some(&params.expected_revision))?;
        let kind = parse_content_kind(&params.kind)?;
        let layout = instance.instance_layout(self.context.minecraft_root.clone());
        let relative = RepositoryPath::new(params.relative_path).map_err(internal_error)?;
        delete_instance_content_with_guard(layout, kind, relative, &repository_guard)
            .await
            .map_err(internal_error)?;
        structured(ContentDelete {
            deleted: true,
            revision: instance_revision(&instance)?,
        })
    }

    #[tool(
        description = "Search Modrinth for content to install",
        output_schema = schema_for_type::<RemoteSearchPage>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true)
    )]
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
        structured(RemoteSearchPage {
            total_hits: page.total_hits,
            offset: page.offset,
            limit: page.limit,
            items: page
                .projects
                .iter()
                .map(RemoteProjectSummary::from)
                .collect(),
        })
    }

    #[tool(
        description = "List installable Modrinth versions for a project",
        output_schema = schema_for_type::<RemoteVersionList>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true)
    )]
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
        structured(RemoteVersionList {
            items: versions.iter().map(RemoteVersionSummary::from).collect(),
        })
    }

    #[tool(
        description = "Install a Modrinth version with dependencies, or return preflight when dry_run is true",
        output_schema = schema_for_type::<ContentInstallResult>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false)
    )]
    async fn content_install(
        &self,
        Parameters(params): Parameters<ContentInstallParams>,
    ) -> Result<CallToolResult, McpError> {
        if params.dry_run != Some(true) {
            self.require_write("content_install")?;
        }
        if params.dry_run == Some(true) {
            let preparation = self.prepare_content_install(&params).await?;
            return structured(ContentInstallResult::Preflight(Box::new(
                (&preparation.preflight).into(),
            )));
        }
        let idempotency_key = params.idempotency_key.clone();
        let (task, replay) = self
            .idempotent_task(
                "content_install",
                idempotency_key.as_deref(),
                || async move {
                    let mut preparation = self.prepare_content_install(&params).await?;
                    if !preparation.preflight.incompatible.is_empty() {
                        let conflicts = preparation
                            .preflight
                            .incompatible
                            .iter()
                            .map(|item| item.title.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        return Err(conflict(format!(
                            "content install blocked by incompatible installed mods: {conflicts}"
                        )));
                    }
                    preparation.request.skip_version_ids = preparation.preflight.skip_version_ids;
                    self.context
                        .tasks
                        .start_content_install_with_dependencies_and_guard(
                            preparation.request,
                            preparation.repository_guard,
                        )
                        .await
                        .map_err(app_error)
                },
            )
            .await?;
        structured(ContentInstallResult::Task(task_started(task, replay)))
    }

    async fn prepare_content_install(
        &self,
        params: &ContentInstallParams,
    ) -> Result<ContentInstallPreparation, McpError> {
        let instance = self.instance(&params.instance_id).await?;
        let repository_guard = self.lock_repository()?;
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
        let request = ContentGraphInstallRequest {
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
            replace: None,
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
        Ok(ContentInstallPreparation {
            request,
            preflight,
            repository_guard,
        })
    }

    #[tool(
        description = "Replace an existing content file with a verified Modrinth version while preserving its path and enable state",
        output_schema = schema_for_type::<ContentUpdateResult>(),
        annotations(read_only_hint = false, destructive_hint = true, idempotent_hint = false)
    )]
    async fn content_update(
        &self,
        Parameters(params): Parameters<ContentUpdateParams>,
    ) -> Result<CallToolResult, McpError> {
        if params.dry_run != Some(true) {
            self.require_write("content_update")?;
        }
        if params.dry_run == Some(true) {
            let preparation = self.prepare_content_update(&params).await?;
            return structured(ContentUpdateResult::Preflight(Box::new(
                ContentUpdatePreflight {
                    dry_run: true,
                    relative_path: preparation.relative_path,
                    preflight: (&preparation.preflight).into(),
                },
            )));
        }
        let idempotency_key = params.idempotency_key.clone();
        let (task, replay) = self
            .idempotent_task(
                "content_update",
                idempotency_key.as_deref(),
                || async move {
                    let mut preparation = self.prepare_content_update(&params).await?;
                    if matches!(
                        preparation.preflight.main.status,
                        ContentPreflightStatus::AlreadyInstalled { .. }
                    ) {
                        return Err(conflict(
                            "the selected version is already installed at this path",
                        ));
                    }
                    if !preparation.preflight.incompatible.is_empty() {
                        let conflicts = preparation
                            .preflight
                            .incompatible
                            .iter()
                            .map(|item| item.title.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        return Err(conflict(format!(
                            "content update blocked by incompatible installed mods: {conflicts}"
                        )));
                    }
                    preparation.request.skip_version_ids = preparation.preflight.skip_version_ids;
                    self.context
                        .tasks
                        .start_content_install_with_dependencies_and_guard(
                            preparation.request,
                            preparation.repository_guard,
                        )
                        .await
                        .map_err(app_error)
                },
            )
            .await?;
        structured(ContentUpdateResult::Task(task_started(task, replay)))
    }

    async fn prepare_content_update(
        &self,
        params: &ContentUpdateParams,
    ) -> Result<ContentUpdatePreparation, McpError> {
        let instance = self.instance(&params.instance_id).await?;
        let repository_guard = self.lock_repository()?;
        self.expect_revision(&instance, params.expected_revision.as_deref())?;
        let kind = parse_content_kind(&params.kind)?;
        let relative_path = RepositoryPath::new(&params.relative_path)
            .map_err(|error| invalid_input(error.to_string()))?;
        let layout = instance.instance_layout(self.context.minecraft_root.clone());
        let library = scan_instance_content(layout.clone())
            .await
            .map_err(app_error)?;
        if !library
            .items_for(kind)
            .any(|item| item.relative_path == relative_path)
        {
            return Err(not_found(format!(
                "content `{}` was not found in instance `{}`",
                params.relative_path, params.instance_id
            )));
        }
        let version = self
            .context
            .modrinth
            .version(
                &params.version_id,
                &tokio_util::sync::CancellationToken::new(),
            )
            .await
            .map_err(internal_error)?;
        let request = ContentGraphInstallRequest {
            instance: layout.clone(),
            kind,
            version: version.clone(),
            display_name: format!("{} {}", version.project_id, version.version_number),
            conflict: ConflictPolicy::Reject,
            game_version: instance.minecraft_version.clone(),
            loader: instance
                .loader
                .as_ref()
                .map(|loader| loader.kind.as_str().to_owned()),
            replace: Some(relative_path),
            skip_version_ids: Vec::new(),
        };
        let installed = tokio::task::spawn_blocking({
            let layout = layout.clone();
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
        Ok(ContentUpdatePreparation {
            request,
            preflight,
            relative_path: params.relative_path.clone(),
            repository_guard,
        })
    }

    async fn launch_account(
        &self,
        selector: Option<&AccountSelectorParams>,
    ) -> Result<LaunchAccount, McpError> {
        let config = self.context.config_store.load().map_err(internal_error)?;
        let (selected, kind) = match selector {
            Some(selector) => (
                selector.username.as_str(),
                parse_account_kind(&selector.kind)?,
            ),
            None => (
                config
                    .selected_account
                    .as_deref()
                    .ok_or_else(|| invalid_input("no account is selected"))?,
                config.selected_account_kind,
            ),
        };
        match kind {
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

    fn lock_repository(&self) -> Result<RepositoryWriteGuard, McpError> {
        try_lock_repository(&self.context.minecraft_root).map_err(app_error)
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
            return Err(revision_conflict(expected, &actual));
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
        F: Future<Output = Result<TaskId, McpError>>,
    {
        let Some(key) = key else {
            return Ok((start().await?, false));
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
        let task = start().await?;
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
            .ok_or_else(|| not_found(format!("unknown instance ID `{id}`")))
    }

    async fn prepare_skin_source(&self, source: &str) -> Result<PreparedSkin, McpError> {
        let source = source.trim();
        if source.is_empty() {
            return Err(invalid_input("skin source cannot be blank"));
        }
        if source.starts_with("https://") || source.starts_with("http://") {
            self.context
                .skin_urls
                .fetch(source, &tokio_util::sync::CancellationToken::new())
                .await
                .map_err(skin_source_error)
        } else if source.contains("://") {
            Err(invalid_input("skin URL must use HTTPS"))
        } else {
            load_skin_file(source).await.map_err(skin_source_error)
        }
    }

    async fn microsoft_account(
        &self,
        username: Option<&str>,
    ) -> Result<MicrosoftAccount, McpError> {
        let config = self.context.config_store.load().map_err(internal_error)?;
        let stored = match username {
            Some(username) => config
                .microsoft_accounts
                .iter()
                .find(|account| account.username == username)
                .ok_or_else(|| {
                    not_found(format!("Microsoft account `{username}` was not found"))
                })?,
            None => {
                if config.selected_account_kind != StoredAccountKind::Microsoft {
                    return Err(invalid_input("no Microsoft account is selected"));
                }
                let selected = config
                    .selected_account
                    .as_deref()
                    .ok_or_else(|| invalid_input("no Microsoft account is selected"))?;
                config
                    .microsoft_accounts
                    .iter()
                    .find(|account| account.username == selected)
                    .ok_or_else(|| {
                        invalid_input("the selected Microsoft account no longer exists")
                    })?
            }
        };
        let uuid = Uuid::parse_str(&stored.uuid).map_err(internal_error)?;
        MicrosoftAccount::new(&stored.username, uuid)
            .map(|account| account.with_skin_url(stored.skin_url.clone()))
            .map_err(internal_error)
    }

    fn skin_entry(&self, entry: &SkinLibraryEntry) -> Result<SkinEntry, McpError> {
        let head_path = self
            .context
            .skin_library
            .head_path(&entry.id)
            .ok_or_else(|| internal_error("skin library contains an invalid content ID"))?;
        Ok(SkinEntry {
            id: entry.id.clone(),
            name: entry.name.clone(),
            variant: entry.variant.as_str().to_owned(),
            source_url: entry.source_url.clone(),
            imported_at: entry.imported_at,
            file_name: entry.file_name.clone(),
            head_path: head_path.display().to_string(),
        })
    }
}

#[tool_handler(
    name = "rmcl",
    instructions = "Read and write RMCL instances, content, Java runtimes, and task lifecycles"
)]
impl ServerHandler for McpServer {}

static IDEMPOTENCY_TASKS: OnceLock<Mutex<BTreeMap<String, TaskId>>> = OnceLock::new();

fn structured(value: impl Serialize) -> Result<CallToolResult, McpError> {
    serde_json::to_value(value)
        .map(CallToolResult::structured)
        .map_err(internal_error)
}

fn task_started(task: TaskId, replay: bool) -> TaskStarted {
    TaskStarted::new(task.get(), replay)
}

fn task_wait_satisfied(snapshot: &rmcl_app::TaskSnapshot) -> bool {
    snapshot.status.is_terminal()
        || (snapshot.kind == rmcl_app::TaskKind::GameLaunch
            && snapshot.phase == Some(TaskPhase::RunningGame))
}

async fn task_wait_result(
    context: &AppContext,
    snapshot: rmcl_app::TaskSnapshot,
    timed_out: bool,
) -> Result<CallToolResult, McpError> {
    let logs = if snapshot.status == TaskStatus::Failed {
        context
            .tasks
            .logs(snapshot.id)
            .await
            .map(|logs| {
                logs.iter()
                    .rev()
                    .take(20)
                    .rev()
                    .map(game_log)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    structured(TaskWaitResult {
        task: task_snapshot(&snapshot),
        timed_out,
        logs,
    })
}

fn parse_skin_variant(value: &str) -> Result<SkinVariant, McpError> {
    match value {
        "classic" => Ok(SkinVariant::Classic),
        "slim" => Ok(SkinVariant::Slim),
        _ => Err(invalid_input(format!(
            "unsupported skin variant `{value}`; expected classic or slim"
        ))),
    }
}

fn microsoft_account_error(error: MicrosoftAccountServiceError) -> McpError {
    match error {
        MicrosoftAccountServiceError::CredentialsMissing => {
            invalid_input("Microsoft account requires sign in")
        }
        other => internal_error(other),
    }
}

fn microsoft_profile(account: &MicrosoftAccount) -> MicrosoftProfile {
    MicrosoftProfile {
        username: account.username().to_owned(),
        uuid: account.uuid().to_string(),
        skin_url: account.skin_url().map(str::to_owned),
        active_variant: account
            .active_skin()
            .map(|skin| skin.variant().as_str().to_owned()),
        skins: account
            .skins()
            .iter()
            .map(|skin| ProfileSkin {
                id: skin.id().to_owned(),
                url: skin.url().to_owned(),
                variant: skin.variant().as_str().to_owned(),
                alias: skin.alias().map(str::to_owned),
                active: skin.is_active(),
            })
            .collect(),
        capes: account
            .capes()
            .iter()
            .map(|cape| ProfileCape {
                id: cape.id().to_owned(),
                url: cape.url().to_owned(),
                alias: cape.alias().map(str::to_owned),
                active: cape.is_active(),
            })
            .collect(),
    }
}

fn skin_library_error(error: SkinLibraryError) -> McpError {
    match error {
        SkinLibraryError::NotFound { id } => not_found(format!("skin `{id}` was not found")),
        SkinLibraryError::Source(error) => skin_source_error(error),
        other => internal_error(other),
    }
}

fn skin_source_error(error: SkinSourceError) -> McpError {
    invalid_input(error.to_string())
}

fn server_bookmark_error(error: ServerBookmarkError) -> McpError {
    match error {
        ServerBookmarkError::Address(error) => invalid_input(error.to_string()),
        ServerBookmarkError::NotFound(id) => not_found(format!("server `{id}` was not found")),
    }
}

fn account_kind(kind: StoredAccountKind) -> AccountKind {
    match kind {
        StoredAccountKind::Offline => AccountKind::Offline,
        StoredAccountKind::Microsoft => AccountKind::Microsoft,
    }
}

fn parse_account_kind(value: &str) -> Result<StoredAccountKind, McpError> {
    match value {
        "offline" => Ok(StoredAccountKind::Offline),
        "microsoft" => Ok(StoredAccountKind::Microsoft),
        _ => Err(invalid_input(format!(
            "unsupported account kind `{value}`; expected offline or microsoft"
        ))),
    }
}

fn account_summaries(config: &LauncherConfig) -> Vec<AccountSummary> {
    config
        .accounts
        .iter()
        .map(|account| AccountSummary {
            username: account.username.clone(),
            uuid: account.uuid.clone(),
            kind: AccountKind::Offline,
            skin_url: None,
            selected: config.selected_account_kind == StoredAccountKind::Offline
                && config.selected_account.as_deref() == Some(account.username.as_str()),
        })
        .chain(
            config
                .microsoft_accounts
                .iter()
                .map(|account| AccountSummary {
                    username: account.username.clone(),
                    uuid: account.uuid.clone(),
                    kind: AccountKind::Microsoft,
                    skin_url: account.skin_url.clone(),
                    selected: config.selected_account_kind == StoredAccountKind::Microsoft
                        && config.selected_account.as_deref() == Some(account.username.as_str()),
                }),
        )
        .collect()
}

fn account_summary(
    config: &LauncherConfig,
    username: &str,
    kind: StoredAccountKind,
) -> Option<AccountSummary> {
    account_summaries(config)
        .into_iter()
        .find(|account| account.username == username && account.kind == account_kind(kind))
}

fn ensure_account_selection(config: &mut LauncherConfig) {
    let selected_exists = match config.selected_account_kind {
        StoredAccountKind::Offline => config.selected_account.as_ref().is_some_and(|selected| {
            config
                .accounts
                .iter()
                .any(|account| &account.username == selected)
        }),
        StoredAccountKind::Microsoft => config.selected_account.as_ref().is_some_and(|selected| {
            config
                .microsoft_accounts
                .iter()
                .any(|account| &account.username == selected)
        }),
    };
    if selected_exists {
        return;
    }
    if let Some(account) = config.accounts.first() {
        config.selected_account = Some(account.username.clone());
        config.selected_account_kind = StoredAccountKind::Offline;
    } else if let Some(account) = config.microsoft_accounts.first() {
        config.selected_account = Some(account.username.clone());
        config.selected_account_kind = StoredAccountKind::Microsoft;
    } else {
        config.selected_account = None;
        config.selected_account_kind = StoredAccountKind::Offline;
    }
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
