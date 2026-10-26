use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon, Sizable, Size, StyledExt,
        button::{Button, ButtonVariants},
        h_flex,
        input::Input,
        scroll::ScrollableElement,
        select::Select,
        spinner::Spinner,
        tab::{Tab, TabBar},
        tag::Tag,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use rmcl_app::{FeaturedServer, ServerStatus, featured_instance_id, featured_servers};
use rmcl_store::StoredServer;

use super::super::{LauncherApp, Page, ServerTab};
use super::common::{card, empty_state, error_banner, page_header, section_title};

impl LauncherApp {
    pub fn render_servers(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_1()
            .min_h_0()
            .p_6()
            .gap_5()
            .child(page_header(Page::Servers, div(), cx))
            .child(
                h_flex().gap_3().items_center().child(
                    TabBar::new("server-tabs")
                        .segmented()
                        .selected_index(self.server_tab.index())
                        .on_click(cx.listener(|this, index: &usize, window, cx| {
                            this.select_server_tab(ServerTab::from_index(*index), window, cx)
                        }))
                        .children(
                            ServerTab::ALL
                                .into_iter()
                                .map(|tab| Tab::new().label(tab.label())),
                        ),
                ),
            )
            .child(match self.server_tab {
                ServerTab::Favorites => self.render_server_favorites(cx).into_any_element(),
                ServerTab::Discover => self.render_server_discover(cx),
            })
    }

    fn server_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let refreshing = match self.server_tab {
            ServerTab::Favorites => self.servers.refreshing,
            ServerTab::Discover => self.servers.featured.refreshing,
        };
        let refresh = Button::new("servers-refresh")
            .outline()
            .icon(IconName::RefreshCw)
            .label(if refreshing {
                "刷新中"
            } else {
                "刷新状态"
            })
            .disabled(refreshing)
            .on_click(cx.listener(|this, _, window, cx| this.refresh_active_servers(window, cx)));
        let add = Button::new("servers-add")
            .primary()
            .icon(IconName::Plus)
            .label("添加服务器")
            .on_click(cx.listener(|this, _, window, cx| this.open_server_dialog(None, window, cx)));

        h_flex()
            .gap_2()
            .child(refresh)
            .when(self.server_tab == ServerTab::Favorites, |this| {
                this.child(add)
            })
    }

    fn render_server_favorites(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
        let has_version = self.selected_version().is_some();
        let has_account = self.selected_account().is_some();
        let can_join = has_version && has_account && self.can_start_operation();
        let join_tooltip = if !has_version {
            "请先选择一个实例"
        } else if !has_account {
            "请先添加一个账户"
        } else if !self.can_start_operation() {
            "当前有任务或游戏正在运行，请先停止"
        } else {
            "使用当前实例和账户进入服务器"
        };

        let mut rows = Vec::new();
        for server in &servers {
            rows.push(self.render_server_row(server, can_join, join_tooltip, cx));
        }

        v_flex()
            .flex_1()
            .min_h_0()
            .gap_5()
            .child(
                card(cx).child(section_title("启动目标", "", cx)).child(
                    h_flex()
                        .gap_3()
                        .child(
                            v_flex()
                                .gap_1()
                                .flex_1()
                                .max_w(px(320.))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("实例"),
                                )
                                .child(
                                    Select::new(&self.version_select)
                                        .placeholder("选择实例")
                                        .accessibility_label("选择实例")
                                        .menu_width(px(280.))
                                        .menu_max_h(px(260.))
                                        .cleanable(false)
                                        .w_full(),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .flex_1()
                                .max_w(px(320.))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("账户"),
                                )
                                .child(
                                    Select::new(&self.account_select)
                                        .placeholder("选择账户")
                                        .accessibility_label("选择账户")
                                        .menu_width(px(280.))
                                        .menu_max_h(px(260.))
                                        .cleanable(false)
                                        .w_full(),
                                ),
                        ),
                ),
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
                            .px_1()
                            .child(
                                div()
                                    .flex_1()
                                    .max_w(px(360.))
                                    .child(Input::new(&self.server_query).w_full()),
                            )
                            .child(self.server_toolbar(cx))
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

    fn render_server_discover(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self.discover_query.read(cx).value().trim().to_lowercase();
        let featured = match featured_servers() {
            Ok(servers) => servers,
            Err(error) => {
                return v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_5()
                    .child(error_banner("无法加载精选服务器", error, cx))
                    .into_any_element();
            }
        };
        let servers = featured
            .iter()
            .filter(|server| {
                query.is_empty()
                    || server.name.to_lowercase().contains(&query)
                    || server.address.to_string().to_lowercase().contains(&query)
                    || server.description.to_lowercase().contains(&query)
                    || server
                        .tags
                        .iter()
                        .any(|tag| tag.to_lowercase().contains(&query))
            })
            .collect::<Vec<_>>();
        let total = featured.len();
        let account = self
            .selected_account()
            .map(|account| account.username().to_owned());
        let can_install = account.is_some() && self.can_start_operation();

        let mut rows = Vec::new();
        for server in &servers {
            rows.push(self.render_featured_server_row(server, can_install, cx));
        }

        v_flex()
            .id("featured-servers-body")
            .flex_1()
            .min_h_0()
            .gap_4()
            .overflow_y_scrollbar()
            .child(
                h_flex()
                    .w_full()
                    .gap_3()
                    .items_center()
                    .px_1()
                    .child(
                        div()
                            .flex_1()
                            .max_w(px(360.))
                            .child(Input::new(&self.discover_query).w_full()),
                    )
                    .child(self.server_toolbar(cx))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("显示 {} / {total} 个服务器", servers.len())),
                    ),
            )
            .when(servers.is_empty(), |this| {
                this.child(empty_state(
                    IconName::Globe,
                    "没有匹配的精选服务器",
                    "尝试清除搜索条件，或稍后刷新。",
                    Button::new("featured-empty-clear")
                        .outline()
                        .label("刷新状态")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.refresh_featured_servers(window, cx)
                        })),
                    cx,
                ))
            })
            .children(rows)
            .into_any_element()
    }

    fn render_featured_server_row(
        &self,
        server: &FeaturedServer,
        can_install: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = self.servers.featured.statuses.get(&server.id);
        let loading = view.is_some_and(|view| view.loading);
        let status = view.and_then(|view| view.status.as_ref());
        let icon_path = view.and_then(|view| view.icon_path.as_deref());
        let instance_id = server
            .version
            .as_deref()
            .map(|version| featured_instance_id(&server.id, version));
        let installed = instance_id.as_ref().is_some_and(|instance_id| {
            self.installed_versions
                .iter()
                .any(|version| version.id.as_str() == instance_id)
        });
        let has_account = self.selected_account().is_some();
        let favorited = self.featured_server_is_favorite(&server.address);
        let favorite_tooltip = if favorited {
            "已收藏；点击取消收藏"
        } else {
            "收藏服务器"
        };
        let favorite_button = Button::new(format!("featured-favorite-{}", server.id))
            .ghost()
            .small()
            .icon(if favorited {
                IconName::StarFill
            } else {
                IconName::Star
            })
            .text_color(if favorited {
                cx.theme().primary
            } else {
                cx.theme().muted_foreground
            })
            .toggled(favorited)
            .tooltip(favorite_tooltip)
            .accessibility_label(favorite_tooltip)
            .on_click(cx.listener({
                let server = server.clone();
                move |this, _, window, cx| {
                    this.toggle_featured_server_favorite(server.clone(), window, cx)
                }
            }));
        let source_button = server.source_url.as_ref().map(|url| {
            let url = url.clone();
            Button::new(format!("featured-source-{}", server.id))
                .ghost()
                .small()
                .icon(IconName::ExternalLink)
                .tooltip("打开官网")
                .accessibility_label("打开官网")
                .on_click(move |_, _, cx| cx.open_url(&url))
        });
        let installable = can_install && server.can_install();
        let install_label = if server.requires_mods {
            "需要整合包"
        } else if installed {
            "进入服务器"
        } else {
            "安装并进入"
        };
        let install_tooltip = if server.requires_mods {
            "需要在服务器页面准备对应的整合包"
        } else if !has_account {
            "请先添加账户"
        } else if !can_install {
            "请等待现有任务结束"
        } else if installed {
            "使用已安装实例进入服务器"
        } else {
            "创建 Vanilla 实例并直接进入服务器"
        };

        let mut tags = Vec::new();
        if server.requires_mods {
            tags.push(Tag::warning().small().child("需要 Mod/整合包"));
        }
        for tag in &server.tags {
            tags.push(Tag::secondary().small().child(tag.clone()));
        }
        let version_note = server.version_note.as_deref().unwrap_or("版本未知");

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
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_base()
                                            .font_semibold()
                                            .truncate()
                                            .child(server.name.clone()),
                                    )
                                    .when_some(server_players(status, cx), |this, players| {
                                        this.child(players)
                                    }),
                            )
                            .child(h_flex().gap_1().flex_wrap().children(tags))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{} · {} · {}",
                                        server.address,
                                        version_note,
                                        server.version.as_deref().unwrap_or("按整合包选择版本")
                                    )),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().foreground)
                                    .child(server.description.clone()),
                            )
                            .child(server_status_line(loading, status, cx)),
                    )
                    .child(
                        v_flex()
                            .w(px(190.))
                            .flex_shrink_0()
                            .gap_1()
                            .items_end()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        Button::new(format!("featured-join-{}", server.id))
                                            .primary()
                                            .icon(if installed {
                                                IconName::Play
                                            } else {
                                                IconName::Download
                                            })
                                            .label(install_label)
                                            .disabled(!installable)
                                            .tooltip(install_tooltip)
                                            .on_click(cx.listener({
                                                let server = server.clone();
                                                move |this, _, window, cx| {
                                                    this.open_featured_install_dialog(
                                                        server.clone(),
                                                        window,
                                                        cx,
                                                    )
                                                }
                                            })),
                                    )
                                    .child(favorite_button),
                            )
                            .when_some(source_button, |this, button| this.child(button)),
                    ),
            )
            .into_any_element()
    }

    fn render_server_row(
        &self,
        server: &StoredServer,
        can_join: bool,
        join_tooltip: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = self.servers.statuses.get(&server.id);
        let loading = view.is_some_and(|view| view.loading);
        let status = view.and_then(|view| view.status.as_ref());
        let icon_path = view.and_then(|view| view.icon_path.as_deref());
        let motd = status
            .map(|status| status.motd.trim())
            .filter(|motd| !motd.is_empty());
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
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_base()
                                            .font_semibold()
                                            .truncate()
                                            .child(server.name.clone()),
                                    )
                                    .when_some(server_players(status, cx), |this, players| {
                                        this.child(players)
                                    }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(server.address.clone()),
                            )
                            .when_some(motd, |this, motd| {
                                this.child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().foreground)
                                        .child(motd.to_owned()),
                                )
                            })
                            .child(server_status_line(loading, status, cx)),
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
                                    .tooltip(join_tooltip)
                                    .on_click(cx.listener({
                                        let id = id.clone();
                                        move |this, _, window, cx| {
                                            this.join_server(id.clone(), window, cx)
                                        }
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
            .into_any_element()
    }
}

fn server_players(status: Option<&ServerStatus>, cx: &App) -> Option<AnyElement> {
    let status = status?;
    let players = match (status.online_players, status.max_players) {
        (Some(online), Some(max)) => format!("{online}/{max} 人"),
        (Some(online), None) => format!("{online} 人"),
        _ => return None,
    };
    Some(
        h_flex()
            .gap_1()
            .flex_shrink_0()
            .items_center()
            .child(
                Icon::new(IconName::Users)
                    .xsmall()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(players),
            )
            .into_any_element(),
    )
}

fn server_status_line(loading: bool, status: Option<&ServerStatus>, cx: &App) -> AnyElement {
    if loading {
        return h_flex()
            .gap_1()
            .items_center()
            .child(
                Spinner::new()
                    .with_size(Size::XSmall)
                    .color(cx.theme().primary),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("查询中"),
            )
            .into_any_element();
    }

    let Some(status) = status else {
        return h_flex()
            .gap_1()
            .items_center()
            .child(
                Icon::new(IconName::CircleX)
                    .xsmall()
                    .text_color(cx.theme().danger),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child("无法连接"),
            )
            .into_any_element();
    };

    let version = status
        .version_name
        .as_deref()
        .filter(|version| !version.is_empty());
    let detail = version.map_or_else(
        || format!("{} ms", status.latency.as_millis()),
        |version| format!("{} ms · {version}", status.latency.as_millis()),
    );

    h_flex()
        .gap_1()
        .items_center()
        .child(
            Icon::new(IconName::CircleCheck)
                .xsmall()
                .text_color(cx.theme().success),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(detail),
        )
        .into_any_element()
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
