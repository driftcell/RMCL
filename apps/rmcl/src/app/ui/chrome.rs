use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Icon, Sizable, Size, StyledExt, TitleBar,
        button::{Button, ButtonVariants},
        h_flex,
        progress::Progress as ProgressBar,
        sidebar::{Sidebar, SidebarGroup, SidebarMenu, SidebarMenuItem},
        spinner::Spinner,
        tag::Tag,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};

use super::super::{
    LauncherApp, Page, account_avatar, task_current_item, task_phase_label, task_progress,
    task_status_label,
};

impl LauncherApp {
    pub fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.log_state.snapshot.as_ref();
        let running = snapshot.is_some_and(|snapshot| {
            matches!(
                snapshot.status,
                rmcl_app::TaskStatus::Running | rmcl_app::TaskStatus::Cancelling
            )
        });
        let dark = cx.theme().mode.is_dark();
        TitleBar::new().child(
            h_flex()
                .w_full()
                .gap_3()
                .pr_2()
                .child(brand_mark(cx))
                .child(div().text_sm().font_semibold().child("RMCL"))
                .child(
                    Tag::new()
                        .outline()
                        .child(env!("CARGO_PKG_VERSION"))
                        .small(),
                )
                .child(div().flex_1())
                .when(running, |this| {
                    this.child(
                        h_flex()
                            .gap_2()
                            .child(
                                Spinner::new()
                                    .with_size(Size::XSmall)
                                    .color(cx.theme().primary),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(task_phase_label(snapshot)),
                            ),
                    )
                })
                .child(
                    Button::new("theme-toggle")
                        .ghost()
                        .small()
                        .icon(if dark { IconName::Sun } else { IconName::Moon })
                        .tooltip("切换明暗主题")
                        .accessibility_label("切换明暗主题")
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_theme(window, cx))),
                ),
        )
    }

    pub fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let running = self.log_state.snapshot.as_ref().is_some_and(|snapshot| {
            matches!(
                snapshot.status,
                rmcl_app::TaskStatus::Running | rmcl_app::TaskStatus::Cancelling
            )
        });
        let account = self.selected_account();
        let account_name = account
            .map(|account| account.username().to_owned())
            .unwrap_or_else(|| "添加账户".to_owned());
        let account_head = account.and_then(|account| self.account_heads.get(&account.uuid()));
        let account_subtitle = match account.map(|account| account.kind()) {
            Some(rmcl_auth::AccountKind::Microsoft) => "正版账户",
            Some(rmcl_auth::AccountKind::Offline) => "离线账户",
            None => "尚未添加账户",
        };
        let selected_is_offline = matches!(account, Some(rmcl_auth::Account::Offline(_)));

        Sidebar::new("main-sidebar")
            .collapsible(false)
            .w(px(232.))
            .child(
                SidebarGroup::new("游戏").child(
                    SidebarMenu::new()
                        .child(self.nav_item(Page::Home, cx))
                        .child(self.nav_item(Page::Versions, cx))
                        .child(self.nav_item(Page::Downloads, cx)),
                ),
            )
            .child(
                SidebarGroup::new("工具").child(
                    SidebarMenu::new().child(
                        SidebarMenuItem::new(Page::Logs.title())
                            .icon(Page::Logs.icon())
                            .active(self.page == Page::Logs)
                            .when(running, |this| {
                                this.suffix(|_, cx| {
                                    Spinner::new()
                                        .with_size(Size::XSmall)
                                        .color(cx.theme().primary)
                                })
                            })
                            .when(!running && self.logs_attention, |this| {
                                this.suffix(|_, cx| {
                                    div().size(px(6.)).rounded_full().bg(cx.theme().danger)
                                })
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.navigate(Page::Logs, window, cx)
                            })),
                    ),
                ),
            )
            .child(
                SidebarGroup::new("应用")
                    .child(SidebarMenu::new().child(self.nav_item(Page::Settings, cx))),
            )
            .footer(
                div().w_full().child(
                    div()
                        .id("account-chip")
                        .w_full()
                        .rounded(cx.theme().radius)
                        .border_1()
                        .border_color(cx.theme().sidebar_border)
                        .bg(cx.theme().sidebar_accent.opacity(0.45))
                        .p_2()
                        .hover(|style| style.bg(cx.theme().sidebar_accent))
                        .on_click(
                            cx.listener(|this, _, window, cx| {
                                this.open_accounts_dialog(window, cx)
                            }),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(account_avatar(account, account_head, 32., cx))
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap_0p5()
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_medium()
                                                .truncate()
                                                .child(account_name),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(account_subtitle),
                                        ),
                                )
                                .child(
                                    Button::new("account-edit")
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Pencil)
                                        .tooltip(if selected_is_offline {
                                            "编辑离线账户"
                                        } else {
                                            "账户"
                                        })
                                        .accessibility_label(if selected_is_offline {
                                            "编辑离线账户"
                                        } else {
                                            "账户"
                                        })
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            cx.stop_propagation();
                                            if selected_is_offline {
                                                this.open_offline_account_dialog(true, window, cx);
                                            } else {
                                                this.open_accounts_dialog(window, cx);
                                            }
                                        })),
                                ),
                        ),
                ),
            )
    }

    fn nav_item(&self, page: Page, cx: &mut Context<Self>) -> SidebarMenuItem {
        SidebarMenuItem::new(page.title())
            .icon(page.icon())
            .active(
                self.page == page
                    || (page == Page::Versions
                        && matches!(
                            self.page,
                            Page::Instance | Page::CreateInstance | Page::ContentBrowser
                        )),
            )
            .on_click(cx.listener(move |this, _, window, cx| this.navigate(page, window, cx)))
    }

    pub fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.log_state.snapshot.as_ref();
        let running = snapshot.is_some_and(|snapshot| {
            matches!(
                snapshot.status,
                rmcl_app::TaskStatus::Running | rmcl_app::TaskStatus::Cancelling
            )
        });
        let can_cancel = self.log_state.can_cancel();
        let (value, loading) = task_progress(snapshot);
        let version_count = self.installed_versions.len();
        let state = if running {
            task_phase_label(snapshot)
        } else {
            task_status_label(snapshot)
        };
        let show_logs = snapshot.is_some_and(|snapshot| {
            matches!(snapshot.status, rmcl_app::TaskStatus::Failed)
                || (snapshot.status == rmcl_app::TaskStatus::Exited
                    && !snapshot.stop_requested
                    && !snapshot.exit.as_ref().is_some_and(|exit| exit.success))
        });
        let current = task_current_item(snapshot);
        let percentage = if loading {
            "准备中".to_owned()
        } else {
            format!("{value:.0}%")
        };

        v_flex()
            .w_full()
            .border_t_1()
            .border_color(cx.theme().status_bar_border)
            .bg(cx.theme().status_bar)
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .when(running, |this| {
                        this.child(
                            Spinner::new()
                                .with_size(Size::XSmall)
                                .color(cx.theme().primary),
                        )
                    })
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_medium()
                            .text_color(cx.theme().foreground)
                            .child(state),
                    )
                    .when(running, |this| {
                        this.child(
                            ProgressBar::new("status-progress")
                                .value(value)
                                .loading(loading)
                                .flex_1()
                                .min_w(px(120.)),
                        )
                        .child(div().flex_shrink_0().child(percentage))
                    })
                    .when(!running, |this| {
                        this.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(format!("{version_count} 个已安装版本")),
                        )
                    })
                    .when(running && can_cancel, |this| {
                        this.child(
                            Button::new("status-stop")
                                .danger()
                                .xsmall()
                                .label("停止")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.cancel(window, cx)),
                                ),
                        )
                    })
                    .when(!running && show_logs, |this| {
                        this.child(
                            Button::new("status-open-logs")
                                .danger()
                                .xsmall()
                                .label("查看日志")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.navigate(Page::Logs, window, cx)
                                })),
                        )
                    })
                    .child(
                        div()
                            .flex_shrink_0()
                            .child(super::super::platform_label(&self.services.platform)),
                    ),
            )
            .when(running && !current.is_empty(), |this| {
                this.child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .gap_2()
                        .px_3()
                        .pb_2()
                        .child(
                            Icon::new(IconName::File)
                                .xsmall()
                                .text_color(cx.theme().muted_foreground),
                        )
                        .child(div().flex_1().min_w_0().truncate().child(current)),
                )
            })
    }
}

fn brand_mark(cx: &App) -> impl IntoElement {
    div()
        .size_5()
        .flex_shrink_0()
        .rounded(px(6.))
        .bg(cx.theme().primary)
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .text_sm()
                .font_semibold()
                .text_color(cx.theme().primary_foreground)
                .child("R"),
        )
}
