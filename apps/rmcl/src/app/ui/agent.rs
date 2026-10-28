//! AI assistant page.

use std::{rc::Rc, time::Duration};

use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon, Sizable, StyledExt,
        bubble::{Bubble, BubbleVariant},
        button::{Button, ButtonVariants},
        h_flex,
        input::Textarea,
        marker::{Marker, MarkerAlignment, MarkerContent, MarkerIcon, MarkerLoadingStyle},
        message::{Message, MessageAlignment, MessageContent},
        message_scroller::MessageScroller,
        progress::Progress,
        text::TextView,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};

use super::super::{
    LauncherApp, Page,
    agent::{
        AgentActivity, AgentKeyState, AgentMessage, AgentReply, AgentRunState, AgentStatusLine,
        AgentStep, AgentStepState,
    },
};
use super::common::{card, error_banner, page_header, section_title};

impl LauncherApp {
    pub fn render_agent(&self, cx: &mut Context<Self>) -> AnyElement {
        let keyless = !self.agent.provider(cx).requires_key();
        let key_missing = !keyless && !matches!(self.agent.settings.key, AgentKeyState::Saved);
        let messages = Rc::new(self.agent.conversation.messages.clone());
        let progress = self.agent.task_progress(self.log_state.snapshot.as_ref());
        let active_task_id = self.agent.active_task_id();
        let activity = self.render_agent_activity(progress, active_task_id, cx);
        let confirmation = self.render_agent_confirmation(cx);
        let running = self.agent.run.is_running();
        let view = cx.entity().downgrade();
        let send_label = if running { "停止" } else { "发送" };
        let send_icon = if running {
            IconName::Square
        } else {
            IconName::Send
        };
        let examples = Rc::new(vec![
            "帮我安装一个 26.3 的 Fabric 实例",
            "给当前实例安装 Sodium",
            "列出已安装的实例和 Java 状态",
        ]);
        let input = self.agent.conversation.input.clone();
        let example_input = self.agent.conversation.input.clone();

        v_flex()
            .flex_1()
            .min_h_0()
            .p_6()
            .gap_4()
            .child(page_header(
                Page::Agent,
                h_flex().gap_2().child(
                    Button::new("agent-clear")
                        .ghost()
                        .small()
                        .icon(IconName::Trash)
                        .label("清空")
                        .disabled(self.agent.run.is_running())
                        .on_click(cx.listener(|this, _, _, cx| this.clear_agent_conversation(cx))),
                ),
                cx,
            ))
            .when_some(self.agent.settings.error.clone(), |this, error| {
                this.child(error_banner("AI 设置不可用", &error, cx))
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .rounded(cx.theme().radius_lg)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background.opacity(0.45))
                    .p_3()
                    .child(
                        div().flex_1().min_h_0().w_full().overflow_hidden().when(
                            self.agent.conversation.messages.is_empty(),
                            |this| {
                                this.child(
                                    v_flex()
                                        .size_full()
                                        .items_center()
                                        .justify_center()
                                        .gap_3()
                                        .child(
                                            div()
                                                .size_12()
                                                .rounded_full()
                                                .bg(cx.theme().primary.opacity(0.12))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .child(
                                                    Icon::new(IconName::Bot)
                                                        .large()
                                                        .text_color(cx.theme().primary),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .text_lg()
                                                .font_semibold()
                                                .child("用自然语言管理 RMCL"),
                                        )
                                        .child(
                                            div()
                                                .max_w_96()
                                                .text_center()
                                                .text_sm()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(
                                                    "Agent 可以查询版本、创建实例、搜索并安装内容，也可以启动游戏。破坏性操作会先询问你。",
                                                ),
                                        )
                                        .when(keyless, |this| {
                                            this.child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child("Kilo 免费模型无需 API Key，可能记录对话；请勿发送敏感信息"),
                                            )
                                        })
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .flex_wrap()
                                                .children(examples.iter().map(|example| {
                                                    let example = (*example).to_owned();
                                                    let example_input = example_input.clone();
                                                    Button::new(format!("agent-example-{example}"))
                                                        .outline()
                                                        .small()
                                                        .label(example.clone())
                                                        .on_click(cx.listener({
                                                            let example = example.clone();
                                                            move |this, _, window, cx| {
                                                                example_input.update(
                                                                    cx,
                                                                    |state, cx| {
                                                                        state.set_value(
                                                                            example.clone(),
                                                                            window,
                                                                            cx,
                                                                        )
                                                                    },
                                                                );
                                                                this.send_agent_message(window, cx);
                                                            }
                                                        }))
                                                })),
                                        )
                                        .when(key_missing, |this| {
                                            this.child(
                                                Button::new("agent-go-settings")
                                                    .primary()
                                                    .label("去设置 API Key")
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            this.navigate(
                                                                Page::Settings,
                                                                window,
                                                                cx,
                                                            )
                                                        },
                                                    )),
                                            )
                                        }),
                                )
                            },
                        )
                        .when(!self.agent.conversation.messages.is_empty(), |this| {
                            this.child(
                                MessageScroller::new(
                                    "agent-messages",
                                    self.agent.conversation.scroller.clone(),
                                    move |index, _, cx| {
                                        let Some(message) = messages.get(index) else {
                                            return div().into_any_element();
                                        };
                                        render_agent_message(index, message, &view, cx)
                                    },
                                )
                                .w_full()
                                .h_full()
                                .min_h_0(),
                            )
                        }),
                    )
                    .child(activity)
                    .child(confirmation),
            )
            .child(
                card(cx)
                    .p_3()
                    .gap_2()
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_2()
                            .child(
                                div().flex_1().min_w_0().child(
                                    Textarea::new(&input)
                                        .w_full()
                                        .aria_label("给 AI 助手发送消息")
                                        .disabled(self.agent.run.is_running()),
                                ),
                            )
                            .child(
                                Button::new("agent-send")
                                    .primary()
                                    .icon(send_icon)
                                    .label(send_label)
                                    .disabled(key_missing && !running)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if this.agent.run.is_running() {
                                            this.agent.stop(cx);
                                        } else {
                                            this.send_agent_message(window, cx);
                                        }
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Enter 发送 · Shift+Enter 换行"),
                            )
                            .child(div().flex_1())
                            .when_some(progress, |this, (value, indeterminate)| {
                                this.child(
                                    div().w_40().child(
                                        Progress::new("agent-compose-progress")
                                            .value(value)
                                            .loading(indeterminate),
                                    ),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_agent_activity(
        &self,
        progress: Option<(f32, bool)>,
        active_task_id: Option<u64>,
        cx: &App,
    ) -> AnyElement {
        let activity = match &self.agent.run {
            AgentRunState::Idle => return div().into_any_element(),
            AgentRunState::Starting { .. } | AgentRunState::Thinking { .. } => {
                return typing_bubble(cx);
            }
            AgentRunState::Working { activity, .. }
            | AgentRunState::Confirming { activity, .. } => activity,
        };
        let mut content = v_flex().w_full().gap_2();
        match activity {
            AgentActivity::Typing => {
                content = content.child(typing_bubble(cx));
            }
            AgentActivity::Status(status) => {
                content = content.child(status_marker(status, true, cx));
            }
            AgentActivity::Tools { status, steps } => {
                if let Some(status) = status {
                    content = content.child(status_marker(status, true, cx));
                }
                content = content.child(v_flex().w_full().gap_1().children(
                    steps.iter().rev().take(3).rev().map(|step| {
                        tool_marker(
                            step,
                            progress.filter(|_| step.task_id == active_task_id),
                            cx,
                        )
                    }),
                ));
            }
        }
        content.into_any_element()
    }

    fn render_agent_confirmation(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some((_, request)) = self.agent.pending_confirmation() else {
            return div().into_any_element();
        };
        card(cx)
            .border_color(cx.theme().warning.opacity(0.45))
            .bg(cx.theme().warning.opacity(0.08))
            .child(section_title(&request.title, &request.body, cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("agent-confirm-cancel")
                            .outline()
                            .label("取消")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.agent.resolve_confirmation(
                                    rmcl_agent::events::ConfirmationDecision::Reject,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("agent-confirm-ok")
                            .danger()
                            .label(request.confirm_label.clone())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.agent.resolve_confirmation(
                                    rmcl_agent::events::ConfirmationDecision::Approve,
                                    cx,
                                );
                            })),
                    ),
            )
            .into_any_element()
    }
}

fn render_agent_message(
    index: usize,
    message: &AgentMessage,
    view: &WeakEntity<LauncherApp>,
    cx: &mut App,
) -> AnyElement {
    match message {
        AgentMessage::User(text) => Message::new()
            .alignment(MessageAlignment::End)
            .id(("agent-user", index))
            .content(
                MessageContent::new().bubble(
                    Bubble::new()
                        .with_variant(BubbleVariant::Filled)
                        .child(text.clone()),
                ),
            )
            .into_any_element(),
        AgentMessage::Assistant(reply) => render_agent_reply(index, reply, view, cx),
        AgentMessage::Notice(text) => Marker::new()
            .alignment(MarkerAlignment::Center)
            .icon(
                MarkerIcon::new().child(
                    Icon::new(IconName::Info)
                        .small()
                        .text_color(cx.theme().muted_foreground),
                ),
            )
            .content(MarkerContent::new().text(text.clone()))
            .into_any_element(),
    }
}

fn render_agent_reply(
    index: usize,
    reply: &AgentReply,
    view: &WeakEntity<LauncherApp>,
    cx: &App,
) -> AnyElement {
    let mut block = v_flex().w_full().gap_2();
    if !reply.steps.is_empty() {
        let open = reply.history_open;
        let summary = reply_summary(reply);
        let failed = reply
            .steps
            .iter()
            .any(|step| step.state == AgentStepState::Failed);
        let icon = if failed {
            IconName::CircleX
        } else {
            IconName::CircleCheck
        };
        let view = view.clone();
        let toggle = Button::new(("agent-reply-history", index))
            .ghost()
            .small()
            .w_full()
            .icon(icon)
            .label(format!(
                "{summary} · {}",
                if open { "收起" } else { "展开" }
            ))
            .on_click(move |_, _, cx| {
                let _ = view.update(cx, |this, cx| {
                    let mut toggled = false;
                    if let Some(AgentMessage::Assistant(reply)) =
                        this.agent.conversation.messages.get_mut(index)
                    {
                        reply.history_open = !reply.history_open;
                        toggled = true;
                    }
                    if toggled {
                        // Expanding or collapsing history changes this row's
                        // height. Without a remeasure the virtual list keeps
                        // the old extent and the wheel stops above the real
                        // end of the transcript.
                        let scroller = this.agent.conversation.scroller.clone();
                        let following = scroller.read(cx).is_following_tail();
                        scroller.update(cx, |scroller, cx| {
                            let _ = scroller.remeasure_items(index..index + 1, cx);
                            if following {
                                scroller.scroll_to_end(cx);
                            }
                        });
                    }
                    cx.notify();
                });
            });
        block = block.child(toggle);
        if open {
            block = block.child(
                v_flex()
                    .w_full()
                    .gap_1()
                    .children(reply.steps.iter().map(|step| tool_marker(step, None, cx))),
            );
        }
    }
    block = block.child(
        Bubble::new()
            .with_variant(BubbleVariant::Ghost)
            .w_full()
            .child(TextView::new(&reply.text).stream_fade(true)),
    );
    Message::new()
        .alignment(MessageAlignment::Start)
        .content(MessageContent::new().child(block))
        .into_any_element()
}

fn reply_summary(reply: &AgentReply) -> String {
    let duration = reply.elapsed.map(|elapsed| {
        let seconds = elapsed.as_secs();
        if seconds >= 60 {
            format!("{} 分 {} 秒", seconds / 60, seconds % 60)
        } else {
            format!("{seconds} 秒")
        }
    });
    match (reply.steps.len(), duration) {
        (count, Some(duration)) => format!("已完成 {count} 个步骤 · {duration}"),
        (count, None) => format!("已完成 {count} 个步骤"),
    }
}

fn status_marker(status: &AgentStatusLine, loading: bool, cx: &App) -> AnyElement {
    let mut marker = Marker::new()
        .loading(loading)
        .with_loading_style(MarkerLoadingStyle::Shimmer)
        .content(MarkerContent::new().text(status.message.clone()));
    if let Some(detail) = &status.detail {
        marker = marker.child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(detail.clone()),
        );
    }
    marker.into_any_element()
}

fn tool_marker(step: &AgentStep, progress: Option<(f32, bool)>, cx: &App) -> AnyElement {
    let (icon, color, loading) = match step.state {
        AgentStepState::Running => (IconName::LoaderCircle, cx.theme().primary, true),
        AgentStepState::Success => (IconName::CircleCheck, cx.theme().success, false),
        AgentStepState::Failed => (IconName::CircleX, cx.theme().danger, false),
        AgentStepState::Skipped => (IconName::CircleStop, cx.theme().muted_foreground, false),
    };
    let label = if step.summary.is_empty() {
        step.label.clone()
    } else {
        format!("{} · {}", step.label, step.summary)
    };
    let marker = Marker::new()
        .id(("agent-step", step.id))
        .role(Role::Status)
        .loading(loading)
        .with_loading_style(MarkerLoadingStyle::Shimmer)
        .icon(MarkerIcon::new().child(Icon::new(icon).small().text_color(color)))
        .content(MarkerContent::new().text(label));
    v_flex()
        .w_full()
        .gap_1()
        .child(marker)
        .when_some(step.error.clone(), |this, error| {
            this.child(
                div()
                    .pl_6()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(error),
            )
        })
        .when_some(
            progress.filter(|_| step.state == AgentStepState::Running),
            |this, (value, indeterminate)| {
                this.child(
                    div().pl_6().child(
                        Progress::new(("agent-step-progress", step.id))
                            .value(value)
                            .loading(indeterminate),
                    ),
                )
            },
        )
        .into_any_element()
}

fn typing_bubble(cx: &App) -> AnyElement {
    let reduce_motion = cx.reduce_motion();
    let dots = h_flex()
        .gap_1()
        .items_center()
        .children((0..3).map(|index| {
            let dot = div()
                .size_1p5()
                .rounded_full()
                .bg(cx.theme().muted_foreground);
            if reduce_motion {
                dot.opacity(0.55).into_any_element()
            } else {
                dot.with_animation(
                    ("agent-typing-dot", index as usize),
                    Animation::new(Duration::from_millis(900)).repeat_synced(),
                    move |dot, phase| {
                        let shifted = (phase + index as f32 / 3.0) % 1.0;
                        let wave = (shifted * std::f32::consts::PI).sin().abs();
                        dot.opacity(0.25 + 0.75 * wave)
                    },
                )
                .into_any_element()
            }
        }));
    Bubble::new()
        .with_variant(BubbleVariant::Secondary)
        .child(dots)
        .into_any_element()
}
