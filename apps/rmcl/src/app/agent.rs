//! Assistant page state and event handling.

use std::time::{Duration, Instant};

use gpui_kit::{
    component::{
        IndexPath,
        input::{InputState, TextareaState},
        message_scroller::MessageScrollerState,
        searchable_list::SearchableVec,
        select::SelectState,
        text::TextViewState,
    },
    *,
};

use rmcl_agent::{
    AgentProvider, AgentSettings, AgentTurnContext,
    events::{AgentEvent, AgentTurnControl, ConfirmationDecision, ConfirmationRequest},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStepState {
    Running,
    Success,
    Failed,
    Skipped,
}

#[derive(Debug, Clone)]
pub struct AgentStep {
    pub id: u64,
    pub label: String,
    pub summary: String,
    pub state: AgentStepState,
    pub error: Option<String>,
    pub task_id: Option<u64>,
}

impl AgentStep {
    pub fn is_running(&self) -> bool {
        self.state == AgentStepState::Running
    }
}

#[derive(Clone)]
pub struct AgentReply {
    pub text: Entity<TextViewState>,
    pub steps: Vec<AgentStep>,
    pub elapsed: Option<Duration>,
    pub history_open: bool,
}

#[derive(Clone)]
pub enum AgentMessage {
    User(String),
    Assistant(AgentReply),
    Notice(String),
}

#[derive(Debug, Clone)]
pub struct AgentStatusLine {
    pub message: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub enum AgentActivity {
    Typing,
    Status(AgentStatusLine),
    Tools {
        status: Option<AgentStatusLine>,
        steps: Vec<AgentStep>,
    },
}

impl AgentActivity {
    pub fn status(&self) -> Option<&AgentStatusLine> {
        match self {
            Self::Typing => None,
            Self::Status(status) => Some(status),
            Self::Tools { status, .. } => status.as_ref(),
        }
    }

    pub fn steps(&self) -> &[AgentStep] {
        match self {
            Self::Tools { steps, .. } => steps,
            _ => &[],
        }
    }

    fn steps_mut(&mut self) -> Option<&mut Vec<AgentStep>> {
        match self {
            Self::Tools { steps, .. } => Some(steps),
            _ => None,
        }
    }

    fn set_status(&mut self, status: AgentStatusLine) {
        match self {
            Self::Typing | Self::Status(_) => *self = Self::Status(status),
            Self::Tools {
                status: current, ..
            } => *current = Some(status),
        }
    }

    fn start_tool(&mut self, step: AgentStep) {
        match self {
            Self::Tools { steps, .. } => steps.push(step),
            _ => {
                *self = Self::Tools {
                    status: self.status().cloned(),
                    steps: vec![step],
                };
            }
        }
    }

    fn finish_tool(
        &mut self,
        id: u64,
        state: AgentStepState,
        error: Option<String>,
        task_id: Option<u64>,
    ) {
        let Some(steps) = self.steps_mut() else {
            return;
        };
        if let Some(step) = steps.iter_mut().find(|step| step.id == id) {
            step.state = state;
            step.error = error;
            step.task_id = task_id;
        }
    }
}

pub enum AgentRunState {
    Idle,
    Starting {
        started_at: Instant,
        cancellation: tokio_util::sync::CancellationToken,
    },
    Thinking {
        started_at: Instant,
        control: AgentTurnControl,
    },
    Working {
        started_at: Instant,
        activity: AgentActivity,
        control: AgentTurnControl,
    },
    Confirming {
        started_at: Instant,
        request_id: u64,
        request: ConfirmationRequest,
        activity: AgentActivity,
        control: AgentTurnControl,
    },
}

impl AgentRunState {
    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    pub fn is_running(&self) -> bool {
        !self.is_idle()
    }

    pub fn control(&self) -> Option<&AgentTurnControl> {
        match self {
            Self::Idle | Self::Starting { .. } => None,
            Self::Thinking { control, .. }
            | Self::Working { control, .. }
            | Self::Confirming { control, .. } => Some(control),
        }
    }

    pub fn activity(&self) -> Option<&AgentActivity> {
        match self {
            Self::Working { activity, .. } | Self::Confirming { activity, .. } => Some(activity),
            Self::Idle | Self::Starting { .. } | Self::Thinking { .. } => None,
        }
    }

    pub fn activity_mut(&mut self) -> Option<&mut AgentActivity> {
        match self {
            Self::Working { activity, .. } | Self::Confirming { activity, .. } => Some(activity),
            Self::Idle | Self::Starting { .. } | Self::Thinking { .. } => None,
        }
    }

    fn started_at(&self) -> Option<Instant> {
        match self {
            Self::Idle => None,
            Self::Starting { started_at, .. } => Some(*started_at),
            Self::Thinking { started_at, .. }
            | Self::Working { started_at, .. }
            | Self::Confirming { started_at, .. } => Some(*started_at),
        }
    }
}

pub enum AgentKeyState {
    Unknown,
    Missing,
    Saved,
    /// The selected provider is a built-in keyless free channel.
    Free,
    Error(String),
}

pub struct AgentSettingsState {
    pub provider_select: Entity<SelectState<SearchableVec<String>>>,
    pub model_input: Entity<InputState>,
    pub base_url_input: Entity<InputState>,
    pub proxy_input: Entity<InputState>,
    pub key_input: Entity<InputState>,
    pub key: AgentKeyState,
    pub provider_error: Option<String>,
    pub error: Option<String>,
    key_generation: u64,
}

pub struct AgentConversation {
    pub messages: Vec<AgentMessage>,
    pub scroller: Entity<MessageScrollerState>,
    pub input: Entity<TextareaState>,
}

pub struct AgentState {
    pub conversation: AgentConversation,
    pub run: AgentRunState,
    pub settings: AgentSettingsState,
    generation: u64,
}

impl super::LauncherApp {
    pub fn clear_agent_conversation(&mut self, cx: &mut Context<Self>) {
        if self.agent.run.is_running() {
            return;
        }
        self.services.agent.clear_history();
        self.agent.clear(cx);
    }

    pub fn send_agent_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.agent.run.is_idle() {
            return;
        }
        let composing = self.agent.conversation.input.update(cx, |state, cx| {
            state.marked_text_range(window, cx).is_some()
        });
        if composing {
            return;
        }
        let input = self
            .agent
            .conversation
            .input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        if input.is_empty() {
            return;
        }
        if let Some(error) = self.agent.settings.provider_error.clone() {
            self.agent.settings.error = Some(error);
            cx.notify();
            return;
        }
        let provider = self.agent.provider(cx);
        if provider.requires_key()
            && matches!(
                self.agent.settings.key,
                AgentKeyState::Unknown | AgentKeyState::Missing | AgentKeyState::Error(_)
            )
        {
            self.agent.settings.error = Some("请先在设置中保存所选服务商的 API Key。".to_owned());
            cx.notify();
            return;
        }
        if let Err(error) = self.save_agent_settings(cx) {
            self.agent.settings.error = Some(error);
            cx.notify();
            return;
        }
        let settings = self.agent.settings(cx);
        let turn_context = AgentTurnContext {
            selected_instance_name: self
                .selected_version()
                .map(|version| version.display_name().to_owned()),
            account_type: match self.selected_account().map(|account| account.kind()) {
                Some(rmcl_auth::AccountKind::Microsoft) => "正版".to_owned(),
                Some(rmcl_auth::AccountKind::Offline) => "离线".to_owned(),
                None => "未登录".to_owned(),
            },
            rmcl_version: env!("CARGO_PKG_VERSION").to_owned(),
            locale: "简体中文".to_owned(),
        };
        self.agent.push_user(input.clone(), cx);
        self.agent
            .conversation
            .input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.agent.generation = self.agent.generation.wrapping_add(1);
        let generation = self.agent.generation;
        let cancellation = tokio_util::sync::CancellationToken::new();
        self.agent.mark_starting(cancellation.clone(), cx);
        let agent = self.services.agent.clone();
        let runtime = self.services.runtime.handle().clone();
        self._agent_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = runtime
                .spawn(async move {
                    agent
                        .start_turn(settings, input, turn_context, cancellation)
                        .await
                })
                .await;
            let Ok(Ok(turn)) = result else {
                let message = match result {
                    Ok(Err(error)) => error.to_string(),
                    Err(error) => format!("后台任务失败：{error}"),
                    Ok(Ok(_)) => unreachable!(),
                };
                let _ = this.update_in(cx, |view, _, cx| {
                    if view.agent.generation != generation {
                        return;
                    }
                    view.agent
                        .conversation
                        .messages
                        .push(AgentMessage::Notice(message));
                    view.agent
                        .conversation
                        .scroller
                        .update(cx, |state, cx| state.append(1, cx));
                    view.agent.finish_turn(cx);
                });
                return;
            };
            let (mut events, control, join_handle) = turn.into_parts();
            let began = this
                .update_in(cx, |view, _, _cx| {
                    if view.agent.generation != generation {
                        control.stop();
                        false
                    } else {
                        view.agent.begin_turn(control);
                        true
                    }
                })
                .unwrap_or(false);
            if !began {
                return;
            }
            while let Some(mut event) = events.recv().await {
                let refresh = matches!(
                    &event,
                    AgentEvent::ToolFinished {
                        name,
                        success: true,
                        skipped: false,
                        ..
                    } if is_agent_write_tool(name)
                );
                let completed_history = match &mut event {
                    AgentEvent::Completed { messages, .. } => Some(std::mem::take(messages)),
                    _ => None,
                };
                let keep_running = this
                    .update_in(cx, |view, window, cx| {
                        if view.agent.generation != generation {
                            return false;
                        }
                        if let Some(messages) = completed_history {
                            view.services.agent.extend_history(messages);
                        }
                        let keep_running = view.agent.handle_event(event, window, cx);
                        if refresh {
                            view.refresh_after_agent_write(window, cx);
                        }
                        keep_running
                    })
                    .unwrap_or(false);
                if !keep_running {
                    break;
                }
            }
            let outcome = join_handle.await;
            let _ = this.update_in(cx, |view, _, cx| {
                if view.agent.generation != generation {
                    return;
                }
                if let Err(error) = outcome
                    && !error.is_cancelled()
                    && !view.agent.run.is_idle()
                {
                    view.agent.conversation.messages.push(AgentMessage::Notice(
                        "AI 后台任务意外结束，请重试。".to_owned(),
                    ));
                    view.agent
                        .conversation
                        .scroller
                        .update(cx, |state, cx| state.append(1, cx));
                }
                if !view.agent.run.is_idle() {
                    view.agent.finish_turn(cx);
                }
            });
        }));
        cx.notify();
    }

    pub fn refresh_after_agent_write(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_external_state(window, cx);
        cx.notify();
    }

    pub fn refresh_agent_key_state(&mut self, cx: &mut Context<Self>) {
        if matches!(self.agent.settings.key, AgentKeyState::Unknown)
            && self._agent_key_task.is_some()
        {
            return;
        }
        let provider = self.agent.provider(cx);
        self.agent.settings.key_generation = self.agent.settings.key_generation.wrapping_add(1);
        let generation = self.agent.settings.key_generation;
        if !provider.requires_key() {
            self._agent_key_task = None;
            self.agent.settings.key = AgentKeyState::Free;
            cx.notify();
            return;
        }
        self.agent.settings.key = AgentKeyState::Unknown;
        let keys = self.services.agent.key_store().clone();
        let runtime = self.services.runtime.handle().clone();
        self._agent_key_task = Some(cx.spawn(async move |this, cx| {
            let result = runtime.spawn_blocking(move || keys.has_key(provider)).await;
            let _ = this.update(cx, |view, cx| {
                if view.agent.settings.key_generation != generation {
                    return;
                }
                view.agent.settings.key = match result {
                    Ok(Ok(true)) => AgentKeyState::Saved,
                    Ok(Ok(false)) => AgentKeyState::Missing,
                    Ok(Err(error)) => AgentKeyState::Error(error.to_string()),
                    Err(error) => AgentKeyState::Error(format!("后台钥匙串任务失败：{error}")),
                };
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub fn set_agent_provider(&mut self, label: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(provider) = AgentProvider::ALL
            .iter()
            .copied()
            .find(|provider| provider.label() == label)
        else {
            return;
        };
        self.agent.settings.model_input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.set_placeholder(provider.default_model(), window, cx);
        });
        self.agent.settings.base_url_input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.set_placeholder(
                provider.default_base_url().unwrap_or("https://..."),
                window,
                cx,
            );
        });
        self.agent.settings.key_input.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        self.agent.settings.provider_error = None;
        self.agent.settings.error = None;
        self._agent_key_task = None;
        self.refresh_agent_key_state(cx);
    }

    pub fn save_agent_settings(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        let provider = self.agent.provider(cx);
        let settings = self.agent.settings(cx);
        settings
            .effective_model()
            .map_err(|error| error.to_string())?;
        settings
            .effective_base_url()
            .map_err(|error| error.to_string())?;
        settings.proxy_url().map_err(|error| error.to_string())?;
        self.config.agent_provider = provider.id().to_owned();
        self.config.agent_model = self
            .agent
            .settings
            .model_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        self.config.agent_base_url =
            non_empty_value(self.agent.settings.base_url_input.read(cx).value().as_ref());
        self.config.agent_proxy =
            non_empty_value(self.agent.settings.proxy_input.read(cx).value().as_ref());
        self.save_config();
        self.agent.settings.provider_error = None;
        self.agent.settings.error = None;
        Ok(())
    }

    pub fn save_agent_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let provider = self.agent.provider(cx);
        let key = self
            .agent
            .settings
            .key_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        if key.is_empty() {
            self.agent.settings.error = Some("API Key 不能为空。".to_owned());
            cx.notify();
            return;
        }
        self.agent.settings.key_generation = self.agent.settings.key_generation.wrapping_add(1);
        let generation = self.agent.settings.key_generation;
        self.agent.settings.key = AgentKeyState::Unknown;
        self.agent.settings.error = None;
        let keys = self.services.agent.key_store().clone();
        let runtime = self.services.runtime.handle().clone();
        self._agent_key_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = runtime
                .spawn_blocking(move || keys.save(provider, &key))
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                if view.agent.settings.key_generation != generation {
                    return;
                }
                match result {
                    Ok(Ok(())) => {
                        view.agent.settings.key_input.update(cx, |state, cx| {
                            state.set_value("", window, cx);
                        });
                        view.agent.settings.key = AgentKeyState::Saved;
                        view.agent.settings.error = None;
                        if let Err(error) = view.save_agent_settings(cx) {
                            view.agent.settings.error = Some(error);
                        }
                    }
                    Ok(Err(error)) => {
                        view.agent.settings.key = AgentKeyState::Error(error.to_string());
                    }
                    Err(error) => {
                        view.agent.settings.key =
                            AgentKeyState::Error(format!("后台钥匙串任务失败：{error}"));
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub fn remove_agent_key(&mut self, cx: &mut Context<Self>) {
        let provider = self.agent.provider(cx);
        self.agent.settings.key_generation = self.agent.settings.key_generation.wrapping_add(1);
        let generation = self.agent.settings.key_generation;
        if !provider.requires_key() {
            self._agent_key_task = None;
            self.agent.settings.key = AgentKeyState::Free;
            cx.notify();
            return;
        }
        self.agent.settings.key = AgentKeyState::Unknown;
        let keys = self.services.agent.key_store().clone();
        let runtime = self.services.runtime.handle().clone();
        self._agent_key_task = Some(cx.spawn(async move |this, cx| {
            let result = runtime.spawn_blocking(move || keys.delete(provider)).await;
            let _ = this.update(cx, |view, cx| {
                if view.agent.settings.key_generation != generation {
                    return;
                }
                view.agent.settings.key = match result {
                    Ok(Ok(())) => AgentKeyState::Missing,
                    Ok(Err(error)) => AgentKeyState::Error(error.to_string()),
                    Err(error) => AgentKeyState::Error(format!("后台钥匙串任务失败：{error}")),
                };
                cx.notify();
            });
        }));
        cx.notify();
    }
}

impl AgentState {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<super::LauncherApp>,
        config: &rmcl_store::LauncherConfig,
    ) -> Self {
        let provider = AgentProvider::from_id(&config.agent_provider).unwrap_or_default();
        let provider_error = AgentProvider::from_id(&config.agent_provider)
            .is_none()
            .then(|| {
                format!(
                    "不支持的 AI 服务商 `{}`，请在设置中重新选择。",
                    config.agent_provider
                )
            });
        let provider_index = AgentProvider::ALL
            .iter()
            .position(|candidate| *candidate == provider)
            .unwrap_or(0);
        let provider_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(
                    AgentProvider::ALL
                        .iter()
                        .map(|provider| provider.label().to_owned())
                        .collect::<Vec<_>>(),
                ),
                Some(IndexPath::default().row(provider_index)),
                window,
                cx,
            )
        });
        let model_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(provider.default_model())
                .default_value(config.agent_model.clone())
        });
        let base_url_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(provider.default_base_url().unwrap_or("https://..."))
                .default_value(config.agent_base_url.clone().unwrap_or_default())
        });
        let proxy_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("可选，例如 socks5://127.0.0.1:7890")
                .default_value(config.agent_proxy.clone().unwrap_or_default())
        });
        let key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("API Key")
                .masked(true)
        });
        // Do not touch the system keyring while constructing the window: an
        // unsigned development build would otherwise prompt for Keychain
        // access on every launch. The key state is refreshed when the AI page
        // or settings page becomes visible.
        let key = AgentKeyState::Unknown;
        let scroller = cx.new(|cx| MessageScrollerState::new(0, cx));
        cx.observe(&scroller, |_, _, cx| cx.notify()).detach();
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 8)
                .submit_on_enter(true)
                .placeholder("告诉 Agent 你想做什么，例如：帮我安装一个 26.3 的 Fabric 实例")
        });
        let conversation = AgentConversation {
            messages: Vec::new(),
            scroller,
            input,
        };
        Self {
            conversation,
            run: AgentRunState::Idle,
            settings: AgentSettingsState {
                provider_select,
                model_input,
                base_url_input,
                proxy_input,
                key_input,
                key,
                provider_error: provider_error.clone(),
                error: provider_error,
                key_generation: 0,
            },
            generation: 0,
        }
    }

    pub fn provider(&self, cx: &Context<super::LauncherApp>) -> AgentProvider {
        let index = self
            .settings
            .provider_select
            .read(cx)
            .selected_index(cx)
            .map(|index| index.row)
            .unwrap_or(0);
        AgentProvider::ALL.get(index).copied().unwrap_or_default()
    }

    pub fn settings(&self, cx: &Context<super::LauncherApp>) -> AgentSettings {
        AgentSettings {
            provider: self.provider(cx),
            model: self.settings.model_input.read(cx).value().trim().to_owned(),
            base_url: non_empty_value(self.settings.base_url_input.read(cx).value().as_ref()),
            proxy: non_empty_value(self.settings.proxy_input.read(cx).value().as_ref()),
        }
    }

    pub fn mark_starting(
        &mut self,
        cancellation: tokio_util::sync::CancellationToken,
        cx: &mut Context<super::LauncherApp>,
    ) {
        self.run = AgentRunState::Starting {
            started_at: Instant::now(),
            cancellation,
        };
        cx.notify();
    }

    pub fn begin_turn(&mut self, control: AgentTurnControl) {
        self.run = AgentRunState::Thinking {
            started_at: Instant::now(),
            control,
        };
    }

    pub fn finish_turn(&mut self, cx: &mut Context<super::LauncherApp>) {
        self.run = AgentRunState::Idle;
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<super::LauncherApp>) {
        if self.run.is_running() {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        self.conversation.messages.clear();
        self.conversation
            .scroller
            .update(cx, |state, cx| state.reset(0, cx));
        cx.notify();
    }

    pub fn push_user(&mut self, text: String, cx: &mut Context<super::LauncherApp>) {
        self.conversation.messages.push(AgentMessage::User(text));
        self.conversation
            .scroller
            .update(cx, |state, cx| state.append(1, cx));
    }

    pub fn handle_event(
        &mut self,
        event: AgentEvent,
        window: &mut Window,
        cx: &mut Context<super::LauncherApp>,
    ) -> bool {
        match event {
            AgentEvent::Started => {
                if matches!(self.run, AgentRunState::Idle) {
                    // The host normally sets the run state before events arrive.
                    return true;
                }
            }
            AgentEvent::Status { message, detail } => {
                let status = AgentStatusLine { message, detail };
                self.update_activity(|activity| activity.set_status(status));
            }
            AgentEvent::ToolStarted {
                id,
                label,
                summary,
                task_id,
                ..
            } => {
                let step = AgentStep {
                    id,
                    label,
                    summary,
                    state: AgentStepState::Running,
                    error: None,
                    task_id,
                };
                self.update_activity(|activity| activity.start_tool(step));
            }
            AgentEvent::ToolFinished {
                id,
                success,
                skipped,
                error,
                task_id,
                ..
            } => {
                let state = if skipped {
                    AgentStepState::Skipped
                } else if success {
                    AgentStepState::Success
                } else {
                    AgentStepState::Failed
                };
                if let Some(activity) = self.run.activity_mut() {
                    activity.finish_tool(id, state, error, task_id);
                }
            }
            AgentEvent::Confirmation {
                request_id,
                request,
            } => {
                let run = std::mem::replace(&mut self.run, AgentRunState::Idle);
                self.run = match run {
                    AgentRunState::Idle => AgentRunState::Idle,
                    AgentRunState::Starting {
                        started_at,
                        cancellation,
                    } => AgentRunState::Starting {
                        started_at,
                        cancellation,
                    },
                    AgentRunState::Thinking {
                        started_at,
                        control,
                    } => AgentRunState::Confirming {
                        started_at,
                        request_id,
                        request,
                        activity: AgentActivity::Typing,
                        control,
                    },
                    AgentRunState::Working {
                        started_at,
                        activity,
                        control,
                    } => AgentRunState::Confirming {
                        started_at,
                        request_id,
                        request,
                        activity,
                        control,
                    },
                    AgentRunState::Confirming {
                        started_at,
                        activity,
                        control,
                        ..
                    } => AgentRunState::Confirming {
                        started_at,
                        request_id,
                        request,
                        activity,
                        control,
                    },
                };
            }
            AgentEvent::ConfirmationExpired { request_id } => {
                self.expire_confirmation(request_id);
            }
            AgentEvent::Completed { markdown, .. } => {
                let elapsed = self.run.started_at().map(|started| started.elapsed());
                let steps = self
                    .run
                    .activity()
                    .map(|activity| activity.steps().to_vec())
                    .unwrap_or_default();
                let history_open = steps
                    .iter()
                    .any(|step| step.state == AgentStepState::Failed);
                let state = cx.new(|cx| TextViewState::markdown(&markdown, cx));
                let index = self.conversation.messages.len();
                self.finish_turn(cx);
                self.conversation
                    .messages
                    .push(AgentMessage::Assistant(AgentReply {
                        text: state.clone(),
                        steps,
                        elapsed,
                        history_open,
                    }));
                // Append first, then observe. The text view notifies while it
                // settles its final layout; observing before the row exists
                // makes `remeasure_items` reject the range and leaves the last
                // row measured too short.
                let scroller = self.conversation.scroller.clone();
                let following = scroller.read(cx).is_following_tail();
                scroller.update(cx, |scroller, cx| {
                    scroller.append(1, cx);
                    let _ = scroller.remeasure_items(index..index + 1, cx);
                    if following {
                        scroller.scroll_to_end(cx);
                    }
                });
                let observed = state;
                cx.observe(&observed, move |this, _, cx| {
                    let scroller = this.agent.conversation.scroller.clone();
                    let following = scroller.read(cx).is_following_tail();
                    scroller.update(cx, |scroller, cx| {
                        let _ = scroller.remeasure_items(index..index + 1, cx);
                        if following {
                            scroller.scroll_to_end(cx);
                        }
                    });
                    cx.notify();
                })
                .detach();
                // The run state changes from Working to Idle above, so the
                // activity row disappears in the same frame. Remeasure once
                // more after that layout settles; otherwise the list can keep
                // the viewport height from the previous frame and clamp the
                // wheel at an offset above the real end.
                let view = cx.entity().downgrade();
                window.on_next_frame(move |_, cx| {
                    let _ = view.update(cx, |this, cx| {
                        let scroller = this.agent.conversation.scroller.clone();
                        let following = scroller.read(cx).is_following_tail();
                        scroller.update(cx, |scroller, cx| {
                            let _ = scroller.remeasure_items(index..index + 1, cx);
                            if following {
                                scroller.scroll_to_end(cx);
                            }
                        });
                        cx.notify();
                    });
                });
            }
            AgentEvent::Failed { message } => {
                self.finish_turn(cx);
                self.conversation
                    .messages
                    .push(AgentMessage::Notice(message));
                self.conversation
                    .scroller
                    .update(cx, |state, cx| state.append(1, cx));
            }
            AgentEvent::Stopped => {
                self.finish_turn(cx);
                self.conversation
                    .messages
                    .push(AgentMessage::Notice("已停止当前回合。".to_owned()));
                self.conversation
                    .scroller
                    .update(cx, |state, cx| state.append(1, cx));
            }
        }
        cx.notify();
        true
    }

    fn update_activity(&mut self, update: impl FnOnce(&mut AgentActivity)) {
        let run = std::mem::replace(&mut self.run, AgentRunState::Idle);
        self.run = match run {
            AgentRunState::Idle => AgentRunState::Idle,
            AgentRunState::Starting {
                started_at,
                cancellation,
            } => AgentRunState::Starting {
                started_at,
                cancellation,
            },
            AgentRunState::Thinking {
                started_at,
                control,
            } => {
                let mut activity = AgentActivity::Typing;
                update(&mut activity);
                AgentRunState::Working {
                    started_at,
                    activity,
                    control,
                }
            }
            AgentRunState::Working {
                started_at,
                mut activity,
                control,
            } => {
                update(&mut activity);
                AgentRunState::Working {
                    started_at,
                    activity,
                    control,
                }
            }
            AgentRunState::Confirming {
                started_at,
                request_id,
                request,
                mut activity,
                control,
            } => {
                update(&mut activity);
                AgentRunState::Confirming {
                    started_at,
                    request_id,
                    request,
                    activity,
                    control,
                }
            }
        };
    }

    fn expire_confirmation(&mut self, request_id: u64) {
        let run = std::mem::replace(&mut self.run, AgentRunState::Idle);
        self.run = match run {
            AgentRunState::Confirming {
                started_at,
                request_id: current_id,
                activity,
                control,
                ..
            } if current_id == request_id => AgentRunState::Working {
                started_at,
                activity,
                control,
            },
            run => run,
        };
    }

    pub fn resolve_confirmation(
        &mut self,
        decision: ConfirmationDecision,
        cx: &mut Context<super::LauncherApp>,
    ) {
        let run = std::mem::replace(&mut self.run, AgentRunState::Idle);
        let AgentRunState::Confirming {
            request_id,
            activity,
            control,
            started_at,
            ..
        } = run
        else {
            self.run = run;
            return;
        };
        let confirmations = control.confirmations.clone();
        self.run = AgentRunState::Working {
            started_at,
            activity,
            control,
        };
        cx.spawn(async move |_, _| {
            confirmations.resolve(request_id, decision).await;
        })
        .detach();
        if decision == ConfirmationDecision::Stop {
            self.stop(cx);
        }
        cx.notify();
    }

    pub fn stop(&mut self, cx: &mut Context<super::LauncherApp>) {
        self.generation = self.generation.wrapping_add(1);
        match &self.run {
            AgentRunState::Starting { cancellation, .. } => cancellation.cancel(),
            _ => {
                if let Some(control) = self.run.control() {
                    control.stop();
                }
            }
        }
        self.finish_turn(cx);
    }

    pub fn pending_confirmation(&self) -> Option<(u64, &ConfirmationRequest)> {
        match &self.run {
            AgentRunState::Confirming {
                request_id,
                request,
                ..
            } => Some((*request_id, request)),
            _ => None,
        }
    }

    pub fn active_task_id(&self) -> Option<u64> {
        self.run.activity().and_then(|activity| {
            activity
                .steps()
                .iter()
                .rev()
                .filter(|step| step.is_running())
                .find_map(|step| step.task_id)
        })
    }

    pub fn task_progress(&self, snapshot: Option<&rmcl_app::TaskSnapshot>) -> Option<(f32, bool)> {
        let task_id = self.active_task_id()?;
        let snapshot = snapshot.filter(|snapshot| snapshot.id.get() == task_id)?;
        Some(super::task_progress(Some(snapshot)))
    }
}

fn non_empty_value(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn is_agent_write_tool(name: &str) -> bool {
    matches!(
        name,
        "instances_create"
            | "instances_update"
            | "instances_repair"
            | "instances_launch"
            | "instances_delete"
            | "content_install"
            | "content_update"
            | "content_set_enabled"
            | "content_delete"
            | "java_ensure"
            | "tasks_cancel"
    )
}

#[cfg(test)]
mod tests {
    use rmcl_app::{TaskId, TaskKind, TaskProgress, TaskSnapshot, TaskStatus};

    fn snapshot(progress: Option<TaskProgress>) -> TaskSnapshot {
        TaskSnapshot {
            id: TaskId::from_raw(1),
            kind: TaskKind::InstanceCreate,
            subject: None,
            status: TaskStatus::Running,
            phase: None,
            progress,
            error: None,
            exit: None,
            stop_requested: false,
            log_lines: 0,
        }
    }

    #[test]
    fn agent_progress_uses_percent_and_indeterminate_semantics() {
        let partial = snapshot(Some(TaskProgress::Processors {
            current: String::new(),
            completed: 1,
            total: 4,
        }));
        assert_eq!(super::super::task_progress(Some(&partial)), (25.0, false));
        assert_eq!(
            super::super::task_progress(Some(&snapshot(None))),
            (0.0, true)
        );
    }
}
