use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon, Sizable, StyledExt,
        button::{Button, ButtonVariants},
        h_flex,
        input::Input,
        scroll::ScrollableElement,
        tab::{Tab, TabBar},
        tag::Tag,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use rmcl_app::{FeaturedServer, featured_instance_id, featured_servers};
use rmcl_store::StoredServer;

use super::super::{LauncherApp, Page, ServerTab};
use super::common::{card, empty_state, error_banner, page_header, section_title, warning_banner};

impl LauncherApp {
    pub fn render_servers(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
        let actions = h_flex().gap_2().child(refresh);
        let actions = match self.server_tab {
            ServerTab::Favorites => actions.child(add),
            ServerTab::Discover => actions,
        };

        v_flex()
            .flex_1()
            .min_h_0()
            .p_6()
            .gap_5()
            .child(page_header(Page::Servers, actions, cx))
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
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
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(match self.server_tab {
                                ServerTab::Favorites => "收藏的服务器",
                                ServerTab::Discover => "精选的热门服务器",
                            }),
                    ),
            )
            .child(match self.server_tab {
                ServerTab::Favorites => self.render_server_favorites(cx).into_any_element(),
                ServerTab::Discover => self.render_server_discover(cx),
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

        let mut rows = Vec::new();
        for server in &servers {
            rows.push(self.render_server_row(server, can_join, cx));
        }

        v_flex()
            .flex_1()
            .min_h_0()
            .gap_5()
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
            .flex_1()
            .min_h_0()
            .gap_5()
            .child(
                card(cx)
                    .child(section_title(
                        "精选服务器",
                        "人工挑选的热门服务器；原版条目可直接安装并进入，需要 Mod 的条目会单独标注。状态由 RMCL 直接查询，不依赖第三方状态接口。",
                        cx,
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Tag::info().child(format!(
                                "账户：{}",
                                account.as_deref().unwrap_or("未选择")
                            )))
                            .child(Tag::secondary().child(format!("共 {total} 个精选服务器")))
                            .child(Tag::new().outline().child("点击“安装并进入”创建隔离实例")),
                    )
                    .child(warning_banner(
                        "第三方服务器",
                        "服务器由各自运营方提供，RMCL 不控制其内容或可用性；公共服务器通常要求正版账号，版本和 Mod 要求也可能变化。",
                        cx,
                    ))
                    .when(account.is_none(), |this| {
                        this.child(warning_banner(
                            "尚未添加账户",
                            "安装完成后需要账户才能进入服务器；请先在侧栏账户卡片中添加账户。",
                            cx,
                        ))
                    }),
            )
            .child(
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
                            .child(
                                div()
                                    .flex_1()
                                    .max_w(px(360.))
                                    .child(Input::new(&self.discover_query).w_full()),
                            )
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
                    .children(rows),
            )
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
        let error = view.and_then(|view| view.error.as_deref());
        let icon_path = view.and_then(|view| view.icon_path.as_deref());
        let status_tag = if loading {
            Tag::info().child("查询中").small()
        } else if status.is_some() {
            Tag::success().child("在线").small()
        } else {
            Tag::danger().child("离线").small()
        };
        let live = status.map(|status| {
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
                            .when_some(live, |this, live| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(live),
                                )
                            }),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .items_end()
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
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(
                                        Button::new(format!("featured-favorite-{}", server.id))
                                            .outline()
                                            .label("收藏")
                                            .on_click(cx.listener({
                                                let server = server.clone();
                                                move |this, _, window, cx| {
                                                    this.favorite_featured_server(
                                                        server.clone(),
                                                        window,
                                                        cx,
                                                    )
                                                }
                                            })),
                                    )
                                    .child(
                                        Button::new(format!("featured-source-{}", server.id))
                                            .ghost()
                                            .icon(IconName::ExternalLink)
                                            .label(server.source_label.clone())
                                            .on_click({
                                                let url = server.source_url.clone();
                                                move |_, _, cx| cx.open_url(&url)
                                            }),
                                    ),
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
