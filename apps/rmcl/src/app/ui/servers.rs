use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon, Sizable, StyledExt,
        button::{Button, ButtonVariants},
        h_flex,
        input::Input,
        scroll::ScrollableElement,
        tag::Tag,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use rmcl_store::StoredServer;

use super::super::{LauncherApp, Page};
use super::common::{card, empty_state, page_header, section_title, warning_banner};

impl LauncherApp {
    pub fn render_servers(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.server_query.read(cx).value().trim().to_lowercase();
        let servers = self
            .config
            .servers
            .iter()
            .filter(|server| {
                query.is_empty()
                    || server.name.to_lowercase().contains(&query)
                    || server.address.to_lowercase().contains(&query)
            })
            .cloned()
            .collect::<Vec<_>>();
        let total = self.config.servers.len();
        let refreshing = self.servers.refreshing;
        let can_join = self.selected_version().is_some()
            && self.selected_account().is_some()
            && self.can_start_operation();
        let instance = self
            .selected_version()
            .map(|version| version.display_name().to_owned())
            .unwrap_or_else(|| "未选择实例".to_owned());
        let account = self
            .selected_account()
            .map(|account| account.username().to_owned())
            .unwrap_or_else(|| "未选择账户".to_owned());

        let add = Button::new("servers-add")
            .primary()
            .icon(IconName::Plus)
            .label("添加服务器")
            .on_click(cx.listener(|this, _, window, cx| this.open_server_dialog(None, window, cx)));
        let refresh = Button::new("servers-refresh")
            .outline()
            .icon(IconName::RefreshCw)
            .label(if refreshing {
                "刷新中"
            } else {
                "刷新全部"
            })
            .disabled(refreshing)
            .on_click(cx.listener(|this, _, window, cx| this.refresh_servers(window, cx)));

        let mut rows = Vec::new();
        for server in &servers {
            rows.push(self.render_server_row(server, can_join, cx));
        }

        v_flex()
            .flex_1()
            .min_h_0()
            .p_6()
            .gap_5()
            .child(page_header(
                Page::Servers,
                h_flex().gap_2().child(refresh).child(add),
                cx,
            ))
            .child(
                card(cx)
                    .child(section_title(
                        "启动目标",
                        "一键进服使用当前选中的实例和账户；服务器收藏是全局的，不绑定实例。",
                        cx,
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Tag::info().child(format!("实例：{instance}")))
                            .child(Tag::secondary().child(format!("账户：{account}")))
                            .child(Tag::new().outline().child(format!(
                                "每 {} 秒自动刷新",
                                super::super::SERVER_REFRESH_INTERVAL.as_secs()
                            ))),
                    )
                    .when(!can_join, |this| {
                        this.child(warning_banner(
                            "一键进服当前不可用",
                            "请先选择实例和账户，并等待现有任务结束。",
                            cx,
                        ))
                    }),
            )
            .child(
                v_flex()
                    .id("servers-body")
                    .flex_1()
                    .min_h_0()
                    .gap_4()
                    .overflow_y_scrollbar()
                    .child(
                        h_flex()
                            .w_full()
                            .gap_3()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .max_w(px(360.))
                                    .child(Input::new(&self.server_query).w_full()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("共 {total} 个服务器")),
                            ),
                    )
                    .when(servers.is_empty(), |this| {
                        this.child(empty_state(
                            IconName::Network,
                            if total == 0 {
                                "还没有服务器"
                            } else {
                                "没有匹配的服务器"
                            },
                            if total == 0 {
                                "添加 Java 版服务器后可以查看 MOTD、在线人数和延迟。"
                            } else {
                                "尝试清除搜索条件。"
                            },
                            Button::new("servers-empty-add")
                                .primary()
                                .icon(IconName::Plus)
                                .label("添加服务器")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_server_dialog(None, window, cx)
                                })),
                            cx,
                        ))
                    })
                    .children(rows),
            )
    }

    fn render_server_row(
        &self,
        server: &StoredServer,
        can_join: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = self.servers.statuses.get(&server.id);
        let loading = view.is_some_and(|view| view.loading);
        let status = view.and_then(|view| view.status.as_ref());
        let error = view.and_then(|view| view.error.as_deref());
        let icon_path = view.and_then(|view| view.icon_path.as_deref());
        let status_tag = if loading {
            Tag::info().child("查询中").small()
        } else if status.is_some() {
            Tag::success().child("在线").small()
        } else {
            Tag::danger().child("离线").small()
        };
        let motd = status
            .map(|status| status.motd.trim())
            .filter(|motd| !motd.is_empty())
            .unwrap_or("没有可用 MOTD");
        let details = status.map(|status| {
            let players = match (status.online_players, status.max_players) {
                (Some(online), Some(max)) => format!("{online}/{max} 人"),
                (Some(online), None) => format!("{online} 人"),
                _ => "人数未知".to_owned(),
            };
            let version = status
                .version_name
                .as_deref()
                .filter(|version| !version.is_empty())
                .unwrap_or("版本未知");
            format!("{players} · {version} · {} ms", status.latency.as_millis())
        });
        let id = server.id.clone();

        card(cx)
            .gap_3()
            .child(
                h_flex()
                    .items_start()
                    .gap_3()
                    .child(server_icon(icon_path, cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_base()
                                            .font_semibold()
                                            .truncate()
                                            .child(server.name.clone()),
                                    )
                                    .child(status_tag),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(server.address.clone()),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().foreground)
                                    .child(motd.to_owned()),
                            )
                            .when_some(details, |this, details| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(details),
                                )
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new(format!("server-join-{id}"))
                                    .primary()
                                    .icon(IconName::Play)
                                    .label("进入")
                                    .disabled(!can_join)
                                    .on_click(cx.listener({
                                        let id = id.clone();
                                        move |this, _, window, cx| {
                                            this.join_server(id.clone(), window, cx)
                                        }
                                    })),
                            )
                            .child(
                                Button::new(format!("server-refresh-{id}"))
                                    .ghost()
                                    .icon(IconName::RefreshCw)
                                    .tooltip("刷新全部服务器状态")
                                    .accessibility_label("刷新全部服务器状态")
                                    .disabled(loading || self.servers.refreshing)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.refresh_servers(window, cx)
                                    })),
                            )
                            .child(
                                Button::new(format!("server-edit-{id}"))
                                    .ghost()
                                    .icon(IconName::Pencil)
                                    .tooltip("编辑服务器")
                                    .accessibility_label("编辑服务器")
                                    .on_click(cx.listener({
                                        let id = id.clone();
                                        move |this, _, window, cx| {
                                            this.open_server_dialog(Some(id.clone()), window, cx)
                                        }
                                    })),
                            )
                            .child(
                                Button::new(format!("server-delete-{id}"))
                                    .ghost()
                                    .icon(IconName::Trash)
                                    .tooltip("删除服务器")
                                    .accessibility_label("删除服务器")
                                    .on_click(cx.listener({
                                        let id = id.clone();
                                        move |this, _, window, cx| {
                                            this.confirm_delete_server(id.clone(), window, cx)
                                        }
                                    })),
                            ),
                    ),
            )
            .when_some(error, |this, error| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(error.to_owned()),
                )
            })
            .into_any_element()
    }
}

fn server_icon(path: Option<&std::path::Path>, cx: &App) -> AnyElement {
    if let Some(path) = path {
        return div()
            .size(px(44.))
            .flex_shrink_0()
            .rounded(cx.theme().radius)
            .overflow_hidden()
            .bg(cx.theme().primary.opacity(0.15))
            .child(
                img(path.to_path_buf())
                    .size_full()
                    .object_fit(ObjectFit::Cover),
            )
            .into_any_element();
    }
    div()
        .size(px(44.))
        .flex_shrink_0()
        .rounded(cx.theme().radius)
        .bg(cx.theme().primary.opacity(0.12))
        .flex()
        .items_center()
        .justify_center()
        .child(Icon::new(IconName::Network).text_color(cx.theme().primary))
        .into_any_element()
}
