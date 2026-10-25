use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon, Sizable, Size, StyledExt,
        button::{Button, ButtonVariants},
        h_flex,
        progress::Progress,
        scroll::ScrollableElement,
        switch::Switch,
        tag::Tag,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use rmcl_app::{TaskSnapshot, TaskStatus};
use rmcl_launch::LogStream;

use super::super::{
    LauncherApp, Page, task_current_item, task_kind_label, task_phase_label, task_progress,
    task_status_label,
};
use super::common::{card, empty_state, page_header};

impl LauncherApp {
    pub fn render_logs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let logs = &self.log_state.logs;
        let running = self.log_state.can_cancel();
        let has_logs = !logs.is_empty();
        let has_task = self.log_state.active_id().is_some();
        let follow_view = cx.entity().downgrade();
        let scroll_view = cx.entity().downgrade();

        v_flex()
            .flex_1()
            .min_h_0()
            .p_6()
            .gap_4()
            .child(page_header(
                Page::Logs,
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("logs-back")
                            .ghost()
                            .small()
                            .icon(IconName::ArrowLeft)
                            .label("返回")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.navigate(Page::Home, window, cx)
                            })),
                    )
                    .child(
                        Switch::new("log-follow")
                            .checked(self.log_follow)
                            .label("自动滚动")
                            .on_change(move |checked, _, cx| {
                                let _ = follow_view.update(cx, |this, cx| {
                                    this.log_follow = *checked;
                                    if *checked {
                                        this.log_scroll.scroll_to_bottom();
                                    }
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        Button::new("logs-copy")
                            .ghost()
                            .small()
                            .icon(IconName::Copy)
                            .tooltip("复制日志")
                            .accessibility_label("复制日志")
                            .disabled(!has_logs)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.copy_logs(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("logs-export")
                            .ghost()
                            .small()
                            .icon(IconName::ArrowDown)
                            .tooltip("导出日志")
                            .accessibility_label("导出日志")
                            .disabled(!has_task)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.export_logs(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("logs-folder")
                            .ghost()
                            .small()
                            .icon(IconName::FolderOpen)
                            .tooltip("打开日志目录")
                            .accessibility_label("打开日志目录")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_logs_dir(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("logs-clear")
                            .ghost()
                            .small()
                            .icon(IconName::Trash)
                            .tooltip("清除当前任务记录")
                            .accessibility_label("清除当前任务记录")
                            .disabled(running)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.clear_logs(window, cx)),
                            ),
                    )
                    .when(running, |this| {
                        this.child(
                            Button::new("logs-stop")
                                .danger()
                                .small()
                                .icon(IconName::Square)
                                .label("停止")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.cancel(window, cx)),
                                ),
                        )
                    }),
                cx,
            ))
            .child(match self.log_state.snapshot.clone() {
                Some(snapshot) => self.render_task_summary(&snapshot, cx).into_any_element(),
                None => card(cx)
                    .child(empty_state(
                        IconName::ScrollText,
                        "还没有任务记录",
                        "安装或启动游戏后显示。",
                        Button::new("logs-install")
                            .primary()
                            .icon(IconName::Download)
                            .label("安装一个版本")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.navigate(Page::Downloads, window, cx)
                            })),
                        cx,
                    ))
                    .into_any_element(),
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .rounded(cx.theme().radius_lg)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().group_box)
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .h(px(38.))
                            .px_4()
                            .justify_between()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().list_head)
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("日志输出")
                            .child(format!("{} 行", logs.len())),
                    )
                    .child(if logs.is_empty() {
                        v_flex()
                            .flex_1()
                            .min_h_0()
                            .items_center()
                            .justify_center()
                            .gap_2()
                            .child(
                                Icon::new(IconName::ScrollText)
                                    .with_size(Size::Large)
                                    .text_color(cx.theme().muted_foreground.opacity(0.5)),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("任务开始后显示游戏输出。"),
                            )
                            .into_any_element()
                    } else {
                        div()
                            .id("log-scroll")
                            .flex_1()
                            .min_h_0()
                            .on_scroll_wheel(move |_, _, cx| {
                                let _ = scroll_view.update(cx, |this, cx| {
                                    if this.log_follow {
                                        this.log_follow = false;
                                        cx.notify();
                                    }
                                });
                            })
                            .track_scroll(&self.log_scroll)
                            .overflow_y_scrollbar()
                            .p_3()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_size(cx.theme().mono_font_size)
                            .children(logs.iter().enumerate().map(|(index, log)| {
                                let stream = match log.stream {
                                    LogStream::Stdout => "out",
                                    LogStream::Stderr => "err",
                                };
                                let color = match log.stream {
                                    LogStream::Stdout => cx.theme().muted_foreground,
                                    LogStream::Stderr => cx.theme().warning,
                                };
                                h_flex()
                                    .w_full()
                                    .items_start()
                                    .gap_2()
                                    .py_0p5()
                                    .child(
                                        div()
                                            .w(px(34.))
                                            .flex_shrink_0()
                                            .text_right()
                                            .text_color(cx.theme().muted_foreground.opacity(0.45))
                                            .child(format!("{:>4}", index + 1)),
                                    )
                                    .child(
                                        div()
                                            .w(px(28.))
                                            .flex_shrink_0()
                                            .text_color(color.opacity(0.75))
                                            .child(stream),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .whitespace_normal()
                                            .text_color(color)
                                            .child(log.text.clone()),
                                    )
                            }))
                            .into_any_element()
                    }),
            )
    }

    fn render_task_summary(&self, snapshot: &TaskSnapshot, cx: &App) -> impl IntoElement {
        let (value, loading) = task_progress(Some(snapshot));
        let current = task_current_item(Some(snapshot));
        let running = matches!(
            snapshot.status,
            TaskStatus::Running | TaskStatus::Cancelling
        );
        card(cx)
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_4()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(status_tag(snapshot.status))
                            .child(div().text_lg().font_semibold().child(format!(
                                "{}任务 #{}",
                                task_kind_label(snapshot.kind),
                                snapshot.id.get()
                            )))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(task_phase_label(Some(snapshot))),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{} 行日志", snapshot.log_lines)),
                    ),
            )
            .when(running || snapshot.progress.is_some(), |this| {
                this.child(
                    Progress::new("task-summary-progress")
                        .value(value)
                        .loading(loading)
                        .w_full(),
                )
            })
            .when(!current.is_empty(), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(current),
                )
            })
            .when_some(snapshot.error.clone(), |this, error| {
                this.child(
                    div()
                        .rounded(cx.theme().radius)
                        .bg(cx.theme().danger.opacity(0.08))
                        .p_3()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
            .when_some(snapshot.exit.clone(), |this, exit| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(match (exit.code, exit.signal) {
                            (Some(code), _) => format!("退出码 {code}"),
                            (_, Some(signal)) => format!("终止信号 {signal}"),
                            _ => "未知退出状态".to_owned(),
                        }),
                )
            })
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(task_status_label(Some(snapshot))),
            )
    }
}

fn status_tag(status: TaskStatus) -> Tag {
    match status {
        TaskStatus::Running | TaskStatus::Cancelling => {
            Tag::info().child(status_label(status)).small()
        }
        TaskStatus::Completed => Tag::success().child(status_label(status)).small(),
        TaskStatus::Cancelled => Tag::warning().child(status_label(status)).small(),
        TaskStatus::Failed => Tag::danger().child(status_label(status)).small(),
        TaskStatus::Exited => Tag::secondary().child(status_label(status)).small(),
    }
}

fn status_label(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Running => "运行中",
        TaskStatus::Cancelling => "正在停止",
        TaskStatus::Completed => "安装完成",
        TaskStatus::Cancelled => "已取消",
        TaskStatus::Failed => "任务失败",
        TaskStatus::Exited => "游戏已退出",
    }
}
