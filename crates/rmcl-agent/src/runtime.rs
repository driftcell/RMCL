//! One-turn Rig agent orchestration over the in-process MCP server.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

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
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use crate::{
    AgentError, AgentKeyStore, AgentSettings,
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
    ) -> Result<AgentTurn, AgentError> {
        let key = self
            .keys
            .load(settings.provider)?
            .ok_or(AgentError::MissingKey(settings.provider))?;
        let model = build_model(&settings, &key)?;
        let history = self.history.lock().await.clone();
        let (event_sender, events) = mpsc::unbounded_channel();
        let confirmations = ConfirmationBroker::default();
        let cancellation = CancellationToken::new();
        let service = self.clone();
        let turn_confirmations = confirmations.clone();
        let turn_cancellation = cancellation.clone();
        let handle = tokio::spawn(async move {
            service
                .run(
                    model,
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

    pub async fn clear_history(&self) {
        self.history.lock().await.clear();
    }

    #[allow(clippy::too_many_arguments)]
    async fn run(
        &self,
        model: DynModel<rig::operation::Completion>,
        input: String,
        history: Vec<Message>,
        turn_context: AgentTurnContext,
        events: mpsc::UnboundedSender<AgentEvent>,
        confirmations: ConfirmationBroker,
        cancellation: CancellationToken,
    ) {
        let _ = events.send(AgentEvent::Started);
        let mcp = match connect_mcp(self.context.clone()).await {
            Ok(mcp) => mcp,
            Err(error) => {
                let _ = events.send(AgentEvent::Failed {
                    message: error.to_string(),
                });
                return;
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
        let result = agent
            .prompt(input)
            .history(history)
            .max_turns(MAX_TURNS)
            .run()
            .await;
        match result {
            Ok(response) => {
                self.history
                    .lock()
                    .await
                    .extend(response.messages().iter().cloned());
                let _ = events.send(AgentEvent::Completed {
                    markdown: response.output(),
                });
            }
            Err(_error) if cancellation.is_cancelled() => {
                let _ = events.send(AgentEvent::Stopped);
            }
            Err(error) => {
                let _ = events.send(AgentEvent::Failed {
                    message: error.to_string(),
                });
            }
        }
        drop(mcp);
    }
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
        let summary = summarize_tool(name, event.tool_args().unwrap_or("{}"));
        let label = tool_label(name);
        let policy = self.policies.get(name).copied();
        let requires_confirmation = policy
            .map(|policy| policy.requires_confirmation(name))
            .unwrap_or(true);
        if requires_confirmation {
            let request = ConfirmationRequest {
                tool: name.to_owned(),
                title: format!("允许执行「{label}」？"),
                body: summary.clone(),
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
        let task_id = serde_json::from_str::<serde_json::Value>(event.tool_args().unwrap_or("{}"))
            .ok()
            .and_then(|value| value.get("task_id").and_then(serde_json::Value::as_u64));
        let _ = self.events.send(AgentEvent::ToolStarted {
            id,
            name: name.to_owned(),
            label: label.to_owned(),
            summary,
            task_id,
        });
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
    let id = text("instance_id")
        .or_else(|| text("task_id"))
        .or_else(|| text("project_id"))
        .or_else(|| text("relative_path"));
    let version = text("base_version_id").or_else(|| text("version_id"));
    let query = text("query").or_else(|| text("game_version"));
    match (name, id, version, query) {
        ("instances_create", Some(id), Some(version), _) => format!("{id} · {version}"),
        ("instances_create", Some(id), None, _) => id.to_owned(),
        ("content_install" | "content_update", Some(path), Some(version), _) => {
            format!("{path} · {version}")
        }
        ("content_install" | "content_update", _, Some(version), _) => version.to_owned(),
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
        assert!(tool_label("tasks_wait").contains("等待"));
    }
}
