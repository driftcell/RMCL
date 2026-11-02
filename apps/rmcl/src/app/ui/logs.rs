use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon, Sizable, Size, StyledExt,
        button::{Button, ButtonVariants},
        h_flex,
        scroll::ScrollableElement,
        switch::Switch,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use rmcl_app::TaskSnapshot;
use rmcl_launch::LogStream;

use super::super::{LauncherApp, Page, task_kind_label};
use super::common::{card, empty_state, page_header};
use super::layout;
use crate::i18n::tr;

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
                            .label(tr("app.common.back"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.navigate(Page::Home, window, cx)
                            })),
                    )
                    .child(
                        Switch::new("log-follow")
                            .checked(self.log_follow)
                            .label(tr("app.logs.auto_scroll"))
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
                            .tooltip(tr("app.logs.copy"))
                            .accessibility_label(tr("app.logs.copy"))
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
                            .tooltip(tr("app.logs.export"))
                            .accessibility_label(tr("app.logs.export"))
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
                            .tooltip(tr("app.logs.open_folder"))
                            .accessibility_label(tr("app.logs.open_folder"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_logs_dir(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("logs-clear")
                            .ghost()
                            .small()
                            .icon(IconName::Trash)
                            .tooltip(tr("app.logs.clear"))
                            .accessibility_label(tr("app.logs.clear"))
                            .disabled(running)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.clear_logs(window, cx)),
                            ),
                    ),
                cx,
            ))
            .child(match self.log_state.snapshot.clone() {
                Some(snapshot) => self.render_task_summary(&snapshot, cx).into_any_element(),
                None => card(cx)
                    .child(empty_state(
                        IconName::ScrollText,
                        tr("app.logs.empty_title"),
                        tr("app.logs.empty_description"),
                        Button::new("logs-install")
                            .primary()
                            .icon(IconName::Download)
                            .label(tr("app.logs.install_version"))
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
                            .h(layout::TABLE_HEADER_HEIGHT)
                            .px_4()
                            .justify_between()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().list_head)
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr("app.logs.output"))
                            .child(
                                rust_i18n::t!("app.logs.line_count", count = logs.len())
                                    .to_string(),
                            ),
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
                                    .child(tr("app.logs.output_empty")),
                            )
                            .into_any_element()
                    } else {
                        div()
                            .id("log-scroll")
                            .flex_1()
                            .min_h_0()
                            .on_scroll(move |_, _, cx| {
                                let _ = scroll_view.update(cx, |this, cx| {
                                    if this.log_follow {
                                        this.log_follow = false;
                                        cx.notify();
                                    }
                                });
                            })
                            .track_scroll(&self.log_scroll)
                            .overflow_y_scrollbar()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_size(cx.theme().mono_font_size)
                            .child(v_flex().p_3().children(logs.iter().enumerate().map(
                                |(index, log)| {
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
                                                .w_8()
                                                .flex_shrink_0()
                                                .text_right()
                                                .text_color(
                                                    cx.theme().muted_foreground.opacity(0.45),
                                                )
                                                .child(format!("{:>4}", index + 1)),
                                        )
                                        .child(
                                            div()
                                                .w_7()
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
                                },
                            )))
                            .into_any_element()
                    }),
            )
    }

    fn render_task_summary(&self, snapshot: &TaskSnapshot, cx: &App) -> impl IntoElement {
        card(cx)
            .child(
                div().text_lg().font_semibold().child(
                    rust_i18n::t!(
                        "app.logs.task_title",
                        kind = task_kind_label(snapshot.kind),
                        id = snapshot.id.get()
                    )
                    .to_string(),
                ),
            )
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
                            (Some(code), _) => {
                                rust_i18n::t!("app.logs.exit_code", code = code).to_string()
                            }
                            (_, Some(signal)) => {
                                rust_i18n::t!("app.logs.termination_signal", signal = signal)
                                    .to_string()
                            }
                            _ => tr("app.common.unknown_exit_status"),
                        }),
                )
            })
    }
}
