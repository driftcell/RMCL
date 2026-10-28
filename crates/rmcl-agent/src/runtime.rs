//! One-turn Rig agent orchestration over the in-process MCP server.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use rig::{
    DynModel,
    agent::{
        AgentBuilder, AgentHook, DispatchAction, DispatchEvent, HookContext, OutcomeAction,
        OutcomeEvent,
    },
    message::Message,
    tool::DynamicTool,
};
use rmcl_app::AppContext;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    AgentError, AgentKeyStore, AgentProvider, AgentSettings,
    events::{
        AgentEvent, AgentTurn, ConfirmationBroker, ConfirmationDecision, ConfirmationRequest,
    },
    mcp::{AgentMcp, ToolPolicy, connect as connect_mcp},
    model::build_model,
};

const MAX_TURNS: usize = 12;
const MAX_TOOL_RESULT_CHARS: usize = 20_000;

/// Host context copied into the system prompt.
#[derive(Debug, Clone)]
pub struct AgentTurnContext {
    pub selected_instance_name: Option<String>,
    pub account_type: String,
    pub rmcl_version: String,
    pub locale: String,
}

impl Default for AgentTurnContext {
    fn default() -> Self {
        Self {
            selected_instance_name: None,
            account_type: "未登录".to_owned(),
            rmcl_version: env!("CARGO_PKG_VERSION").to_owned(),
            locale: "简体中文".to_owned(),
        }
    }
}

/// Shared assistant service owned by the application, never by a page.
pub struct AgentService {
    context: Arc<AppContext>,
    keys: AgentKeyStore,
    history: Mutex<Vec<Message>>,
}

impl AgentService {
    pub fn new(context: Arc<AppContext>, keys: AgentKeyStore) -> Self {
        Self {
            context,
            keys,
            history: Mutex::new(Vec::new()),
        }
    }

    pub fn key_store(&self) -> &AgentKeyStore {
        &self.keys
    }

    pub async fn start_turn(
        self: &Arc<Self>,
        settings: AgentSettings,
        input: String,
        turn_context: AgentTurnContext,
        cancellation: CancellationToken,
    ) -> Result<AgentTurn, AgentError> {
        if cancellation.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        let key = if settings.provider.requires_key() {
            self.keys
                .load(settings.provider)?
                .ok_or(AgentError::MissingKey(settings.provider))?
        } else {
            // Kilo's anonymous free pool accepts unauthenticated requests. Rig
            // omits the Authorization header when the API key is empty.
            String::new()
        };
        let model = build_model(&settings, &key)?;
        if cancellation.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        let provider = settings.provider;
        let history = self.lock_history().clone();
        let (event_sender, events) = mpsc::unbounded_channel();
        let confirmations = ConfirmationBroker::default();
        let service = self.clone();
        let turn_confirmations = confirmations.clone();
        let turn_cancellation = cancellation.clone();
        let handle = tokio::spawn(async move {
            service
                .run(
                    model,
                    provider,
                    key,
                    input,
                    history,
                    turn_context,
                    event_sender,
                    turn_confirmations,
                    turn_cancellation,
                )
                .await;
        });
        Ok(AgentTurn::new(events, confirmations, cancellation, handle))
    }

    pub fn clear_history(&self) {
        self.lock_history().clear();
    }

    /// Commits a completed transcript after the host accepts its completion event.
    pub fn extend_history(&self, messages: impl IntoIterator<Item = Message>) {
        self.lock_history().extend(messages);
    }

    fn lock_history(&self) -> MutexGuard<'_, Vec<Message>> {
        // No await happens while this lock is held, so clearing the visible
        // conversation and starting its next turn can share one synchronous
        // ordering point on the UI thread.
        self.history
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[allow(clippy::too_many_arguments)]
    async fn run(
        &self,
        model: DynModel<rig::operation::Completion>,
        provider: AgentProvider,
        api_key: String,
        input: String,
        history: Vec<Message>,
        turn_context: AgentTurnContext,
        events: mpsc::UnboundedSender<AgentEvent>,
        confirmations: ConfirmationBroker,
        cancellation: CancellationToken,
    ) {
        let _ = events.send(AgentEvent::Started);
        let mcp = tokio::select! {
            _ = cancellation.cancelled() => {
                let _ = events.send(AgentEvent::Stopped);
                return;
            }
            result = connect_mcp(self.context.clone()) => match result {
                Ok(mcp) => mcp,
                Err(error) => {
                    let _ = events.send(AgentEvent::Failed {
                        message: error.to_string(),
                    });
                    return;
                }
            }
        };
        let status_events = events.clone();
        mcp.status_sink.set(Arc::new(move |message, detail| {
            let _ = status_events.send(AgentEvent::Status { message, detail });
        }));

        let tools = match dynamic_tools(&mcp) {
            Ok(tools) => tools,
            Err(error) => {
                let _ = events.send(AgentEvent::Failed {
                    message: error.to_string(),
                });
                return;
            }
        };
        let hook = PermissionHook {
            policies: mcp.policies.clone(),
            events: events.clone(),
            confirmations,
            cancellation: cancellation.clone(),
        };
        let agent = AgentBuilder::new(model)
            .preamble(system_prompt(&turn_context))
            .dynamic_tools(tools)
            .add_hook(hook)
            .build();
        let result = tokio::select! {
            _ = cancellation.cancelled() => {
                let _ = events.send(AgentEvent::Stopped);
                return;
            }
            result = agent
                .prompt(input)
                .history(history)
                .max_turns(MAX_TURNS)
                .run() => result
        };
        match result {
            Ok(response) => {
                let _ = events.send(AgentEvent::Completed {
                    markdown: response.output(),
                    messages: response.messages().to_vec(),
                });
            }
            Err(_error) if cancellation.is_cancelled() => {
                let _ = events.send(AgentEvent::Stopped);
            }
            Err(error) => {
                let message = redact_provider_error(&error.to_string(), &api_key);
                let _ = events.send(AgentEvent::Failed {
                    message: friendly_error(provider, &message),
                });
            }
        }
        drop(mcp);
    }
}

fn redact_provider_error(error: &str, api_key: &str) -> String {
    if api_key.is_empty() {
        return error.to_owned();
    }
    let redacted = error.replace(api_key, "[redacted]");
    let encoded = url::form_urlencoded::byte_serialize(api_key.as_bytes()).collect::<String>();
    if encoded == api_key {
        redacted
    } else {
        redacted.replace(&encoded, "[redacted]")
    }
}

/// Turns provider-specific free-tier failures into actionable UI text. Paid
/// providers keep their original error because the message often contains the
/// exact quota or billing detail the user needs.
fn friendly_error(provider: AgentProvider, error: &str) -> String {
    if provider != AgentProvider::Kilo {
        return error.to_owned();
    }
    let lower = error.to_ascii_lowercase();
    if lower.contains("429") || lower.contains("rate limit") || lower.contains("too many requests")
    {
        return "Kilo 免费通道当前请求过多，请稍后再试，或在设置中配置自己的 API Key。".to_owned();
    }
    if lower.contains("401") || lower.contains("invalid_token") {
        return "Kilo 免费通道暂时不可用，请稍后重试，或在设置中改用其他服务商。".to_owned();
    }
    error.to_owned()
}

fn dynamic_tools(mcp: &AgentMcp) -> Result<Vec<DynamicTool>, AgentError> {
    mcp.tools
        .iter()
        .cloned()
        .map(|tool| {
            let name = tool.name().to_owned();
            let tool = if name == "tasks_wait" {
                tool.with_timeout(Some(Duration::from_secs(660)))
            } else {
                tool
            };
            DynamicTool::try_from(tool).map_err(|error| {
                AgentError::ToolConversion(format!("failed to expose MCP tool `{name}`: {error}"))
            })
        })
        .collect()
}

struct PermissionHook {
    policies: BTreeMap<String, ToolPolicy>,
    events: mpsc::UnboundedSender<AgentEvent>,
    confirmations: ConfirmationBroker,
    cancellation: CancellationToken,
}

impl AgentHook for PermissionHook {
    async fn on_dispatch(&self, _ctx: &HookContext, event: DispatchEvent<'_>) -> DispatchAction {
        let Some(name) = event.tool_name() else {
            return DispatchAction::proceed();
        };
        if self.cancellation.is_cancelled() {
            return DispatchAction::stop("用户停止了任务");
        }
        if name == "agent_report_status" {
            return DispatchAction::proceed();
        }
        let id = event.id.as_u64();
        let args = event.tool_args().unwrap_or("{}");
        let summary = summarize_tool(name, args);
        let label = tool_label(name);
        let args = serde_json::from_str::<serde_json::Value>(args).unwrap_or_default();
        let dry_run = args
            .get("dry_run")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let policy = self.policies.get(name).copied();
        let requires_confirmation = policy
            .map(|policy| policy.requires_confirmation(name, dry_run))
            .unwrap_or(true);
        let task_id = args.get("task_id").and_then(serde_json::Value::as_u64);
        let _ = self.events.send(AgentEvent::ToolStarted {
            id,
            name: name.to_owned(),
            label: label.to_owned(),
            summary: summary.clone(),
            task_id,
        });
        if requires_confirmation {
            let request = ConfirmationRequest {
                tool: name.to_owned(),
                title: format!("允许执行「{label}」？"),
                body: summary,
                confirm_label: confirm_label(name).to_owned(),
            };
            match self.confirmations.ask(&self.events, request).await {
                ConfirmationDecision::Approve => {}
                ConfirmationDecision::Reject => {
                    let _ = self.events.send(AgentEvent::ToolFinished {
                        id,
                        name: name.to_owned(),
                        success: false,
                        skipped: true,
                        error: None,
                        task_id: None,
                    });
                    return DispatchAction::skip("用户拒绝了此操作");
                }
                ConfirmationDecision::Stop => return DispatchAction::stop("用户停止了任务"),
            }
        }
        DispatchAction::proceed()
    }

    async fn on_outcome(&self, _ctx: &HookContext, event: OutcomeEvent<'_>) -> OutcomeAction {
        let Some(name) = event.tool_name() else {
            return OutcomeAction::proceed();
        };
        if name == "agent_report_status" {
            return OutcomeAction::proceed();
        }
        let Some(result) = event.tool_result() else {
            return OutcomeAction::proceed();
        };
        let task_id = result
            .output()
            .as_json()
            .and_then(|value| value.get("task_id"))
            .and_then(serde_json::Value::as_u64);
        let _ = self.events.send(AgentEvent::ToolFinished {
            id: event.id.as_u64(),
            name: name.to_owned(),
            success: result.is_success(),
            skipped: result.is_skipped(),
            error: result.error().map(ToString::to_string),
            task_id,
        });
        let rendered = result.output().render();
        if rendered.chars().count() > MAX_TOOL_RESULT_CHARS {
            let truncated = rendered
                .chars()
                .take(MAX_TOOL_RESULT_CHARS)
                .collect::<String>();
            return OutcomeAction::rewrite_tool_result(
                &event,
                format!("{truncated}\n\n[tool result truncated by RMCL]"),
            );
        }
        OutcomeAction::proceed()
    }
}

fn system_prompt(context: &AgentTurnContext) -> String {
    const TEMPLATE: &str = include_str!("../prompts/system.md");
    TEMPLATE
        .replace("{date}", &current_date())
        .replace("{os}", std::env::consts::OS)
        .replace("{arch}", std::env::consts::ARCH)
        .replace("{rmcl_version}", &context.rmcl_version)
        .replace(
            "{selected_instance_name}",
            context.selected_instance_name.as_deref().unwrap_or("无"),
        )
        .replace("{account_type}", &context.account_type)
        .replace("{locale}", &context.locale)
}

fn current_date() -> String {
    // The std library has no calendar formatting; a compact UTC date is
    // sufficient context for the model.
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let days = seconds / 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!("{year:04}-{month:02}-{day:02} (UTC)")
}

// Howard Hinnant's civil_from_days algorithm, kept local to avoid a date
// dependency for one prompt line.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m as u32, d as u32)
}

fn tool_label(name: &str) -> &'static str {
    match name {
        "instances_list" => "查看实例",
        "instances_get" => "查看实例详情",
        "instances_create" => "创建实例",
        "instances_update" => "修改实例",
        "instances_repair" => "修复实例",
        "instances_launch" => "启动游戏",
        "instances_delete" => "删除实例",
        "versions_list" => "查询游戏版本",
        "loaders_list" => "查询加载器版本",
        "content_list" => "查看本地内容",
        "content_search" => "搜索 Modrinth",
        "content_versions" => "查看可用版本",
        "content_install" => "安装内容",
        "content_update" => "更新内容",
        "content_set_enabled" => "启用或禁用内容",
        "content_delete" => "删除内容",
        "java_list" => "查看 Java",
        "java_ensure" => "准备 Java",
        "accounts_list" => "查看账户",
        "servers_list" => "查看服务器",
        "servers_ping" => "查询服务器状态",
        "skins_list" => "查看皮肤库",
        "skins_profile" => "查看正版皮肤",
        "tasks_list" => "查看任务",
        "tasks_get" => "查看任务状态",
        "tasks_wait" => "等待任务完成",
        "tasks_logs" => "读取任务日志",
        "tasks_cancel" => "取消任务",
        "agent_report_status" => "更新状态",
        _ => "调用工具",
    }
}

fn confirm_label(name: &str) -> &'static str {
    match name {
        "instances_delete" | "content_delete" | "accounts_delete" | "servers_delete"
        | "skins_delete" => "删除",
        "instances_launch" => "启动",
        "content_install" => "安装",
        "content_update" | "skins_apply" | "skins_reset" => "继续",
        "tasks_cancel" => "取消任务",
        _ => "继续",
    }
}

fn summarize_tool(name: &str, args: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(args).unwrap_or_default();
    if value.as_object().is_some_and(|object| object.is_empty()) {
        return String::new();
    }
    let text = |key: &str| value.get(key).and_then(serde_json::Value::as_str);
    let version = text("base_version_id").or_else(|| text("version_id"));
    if matches!(
        name,
        "content_install" | "content_update" | "content_set_enabled" | "content_delete"
    ) {
        let target = text("relative_path")
            .or_else(|| text("project_id"))
            .or_else(|| text("instance_id"));
        let instance = text("instance_id");
        return match (target, version, instance) {
            (Some(target), Some(version), Some(instance)) => {
                format!("{target} · {version} · {instance}")
            }
            (Some(target), Some(version), None) => format!("{target} · {version}"),
            (Some(target), None, Some(instance)) => format!("{target} · {instance}"),
            (Some(target), None, None) => target.to_owned(),
            (None, Some(version), Some(instance)) => format!("{version} · {instance}"),
            (None, Some(version), None) => version.to_owned(),
            (None, None, _) => truncate(args, 120),
        };
    }
    let id = text("instance_id")
        .or_else(|| text("task_id"))
        .or_else(|| text("project_id"))
        .or_else(|| text("relative_path"));
    let query = text("query").or_else(|| text("game_version"));
    match (name, id, version, query) {
        ("instances_create", Some(id), Some(version), _) => format!("{id} · {version}"),
        ("instances_create", Some(id), None, _) => id.to_owned(),
        ("content_search" | "versions_list" | "loaders_list", _, _, Some(query)) => {
            query.to_owned()
        }
        (_, Some(id), _, _) => id.to_owned(),
        _ => truncate(args, 120),
    }
}

fn truncate(value: &str, max: usize) -> String {
    let mut chars = value.chars();
    let text = chars.by_ref().take(max).collect::<String>();
    if chars.next().is_some() {
        format!("{text}…")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_contains_language_and_context() {
        let prompt = system_prompt(&AgentTurnContext {
            selected_instance_name: Some("fabric-1.21".to_owned()),
            account_type: "正版".to_owned(),
            rmcl_version: "0.9.0".to_owned(),
            locale: "简体中文".to_owned(),
        });
        assert!(prompt.contains("默认用简体中文"));
        assert!(prompt.contains("fabric-1.21"));
        assert!(prompt.contains("正版"));
        assert!(prompt.contains("0.9.0"));
    }

    #[test]
    fn tool_summaries_prefer_stable_names() {
        assert_eq!(
            summarize_tool(
                "instances_create",
                r#"{"instance_id":"26.3","base_version_id":"26.3"}"#
            ),
            "26.3 · 26.3"
        );
        assert_eq!(summarize_tool("java_list", "{}"), "");
        assert_eq!(
            summarize_tool("content_search", r#"{"query":"sodium"}"#),
            "sodium"
        );
        assert_eq!(
            summarize_tool(
                "content_delete",
                r#"{"instance_id":"fabric","relative_path":"mods/sodium.jar"}"#
            ),
            "mods/sodium.jar · fabric"
        );
        assert_eq!(
            summarize_tool(
                "content_update",
                r#"{"instance_id":"fabric","relative_path":"mods/sodium.jar","version_id":"v2"}"#
            ),
            "mods/sodium.jar · v2 · fabric"
        );
        assert_eq!(
            summarize_tool(
                "content_install",
                r#"{"instance_id":"fabric","project_id":"sodium","version_id":"v2"}"#
            ),
            "sodium · v2 · fabric"
        );
        assert!(tool_label("tasks_wait").contains("等待"));
    }

    #[test]
    fn provider_errors_redact_raw_and_encoded_api_keys() {
        let error =
            "error sending request for url (https://example.com/v1beta/models/x?key=secret/key)";
        let redacted = redact_provider_error(error, "secret/key");
        assert!(!redacted.contains("secret/key"));
        assert!(redacted.contains("[redacted]"));
        let encoded = redact_provider_error(
            "error sending request for url (https://example.com/?key=secret%2Fkey)",
            "secret/key",
        );
        assert!(!encoded.contains("secret%2Fkey"));
        assert!(encoded.contains("[redacted]"));
    }

    #[test]
    fn free_tier_errors_are_actionable() {
        let error = "CompletionError: HttpError: 429 Too Many Requests";
        assert!(friendly_error(AgentProvider::Kilo, error).contains("免费通道"));
        assert_eq!(friendly_error(AgentProvider::OpenAi, error), error);
    }

    fn service(temporary: &tempfile::TempDir) -> Arc<AgentService> {
        use rmcl_platform::MemorySecretStore;
        use rmcl_store::{ConfigStore, LauncherConfig};

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
        Arc::new(AgentService::new(
            context,
            AgentKeyStore::new(Arc::new(MemorySecretStore::default())),
        ))
    }

    #[test]
    fn clear_history_discards_model_context() {
        let temporary = tempfile::TempDir::new().unwrap();
        let service = service(&temporary);
        service.lock_history().push(Message::user("old context"));

        service.clear_history();

        assert!(service.lock_history().is_empty());
    }

    #[tokio::test]
    async fn cancellation_before_start_prevents_model_or_mcp_work() {
        let temporary = tempfile::TempDir::new().unwrap();
        let service = service(&temporary);
        let settings = AgentSettings::from_config("kilo", "", None, None).unwrap();
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let result = service
            .start_turn(
                settings,
                "hello".to_owned(),
                AgentTurnContext::default(),
                cancellation,
            )
            .await;

        assert!(matches!(result, Err(AgentError::Cancelled)));
        let service = match Arc::try_unwrap(service) {
            Ok(service) => service,
            Err(_) => panic!("agent service should have one owner"),
        };
        let context = match Arc::try_unwrap(service.context) {
            Ok(context) => context,
            Err(_) => panic!("app context should have no remaining owners"),
        };
        context.runtime.shutdown_background();
    }
}
