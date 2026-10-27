use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon, Sizable, Size, StyledExt,
        button::{Button, ButtonVariants},
        h_flex,
        scroll::ScrollableElement,
        select::Select,
        tag::Tag,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use rmcl_app::InstalledVersion;

use super::super::{LauncherApp, Page, ServerTab, short_date, version_tile, version_type_label};
use super::common::{card, page_header, section_title, server_tile, type_tag};
use super::layout;
use super::servers::{server_players, server_status_line};

impl LauncherApp {
    pub fn render_home(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.selected_version();
        let version_label = selected
            .map(|version| version.display_name().to_owned())
            .unwrap_or_else(|| "准备安装你的第一个实例".to_owned());
        let version_meta = selected
            .map(|version| {
                let base = format!(
                    "{} · {}",
                    version_type_label(version.version_type.as_deref()),
                    short_date(version.release_time.as_deref())
                );
                if version.display_name() != version.id.as_str() {
                    format!("{} · Minecraft {}", base, version.minecraft_version)
                } else {
                    base
                }
            })
            .unwrap_or_else(|| "从下载页选择任意 Vanilla 版本开始。".to_owned());
        let has_account = self.selected_account().is_some();
        let can_start = self.can_start_operation();

        v_flex()
            .flex_1()
            .min_h_0()
            .p_6()
            .gap_6()
            .child(page_header(Page::Home, div(), cx))
            .child(
                v_flex()
                    .id("home-body")
                    .flex_1()
                    .min_h_0()
                    .gap_6()
                    .when_some(self.config_warning.clone(), |this, warning| {
                        this.child(error_banner("配置读取失败", &warning, cx))
                    })
                    .when_some(self.config_save_error.clone(), |this, error| {
                        this.child(error_banner("配置保存失败", &error, cx))
                    })
                    .child(self.render_hero(
                        selected,
                        &version_label,
                        &version_meta,
                        has_account,
                        can_start,
                        cx,
                    ))
                    .child(self.render_quick_connect(cx))
                    .child(
                        h_flex()
                            .w_full()
                            .items_start()
                            .gap_4()
                            .child(
                                v_flex()
                                    .flex_basis(rems(0.))
                                    .flex_grow(2.0)
                                    .min_w_0()
                                    .child(self.render_recent_versions(can_start, cx)),
                            )
                            .child(
                                v_flex()
                                    .flex_basis(rems(0.))
                                    .flex_grow(1.0)
                                    .min_w_0()
                                    .child(self.render_quick_actions(cx)),
                            ),
                    )
                    .overflow_y_scrollbar(),
            )
    }

    fn render_hero(
        &self,
        selected: Option<&InstalledVersion>,
        version_label: &str,
        version_meta: &str,
        has_account: bool,
        can_start: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .relative()
            .w_full()
            .overflow_hidden()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().primary.opacity(0.18))
            .bg(cx.theme().primary.opacity(0.055))
            .p_6()
            .child(
                h_flex()
                    .items_start()
                    .gap_6()
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .items_start()
                            .gap_5()
                            .child(hero_tile(selected.is_some(), cx))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_2()
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(
                                                Tag::new()
                                                    .child(if selected.is_some() {
                                                        "当前实例"
                                                    } else {
                                                        "尚未安装"
                                                    })
                                                    .small(),
                                            )
                                            .when_some(selected, |this, version| {
                                                this.child(if version.is_isolated() {
                                                    Tag::secondary().child("隔离").small()
                                                } else {
                                                    Tag::new().outline().child("共享").small()
                                                })
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_3xl()
                                            .font_semibold()
                                            .line_height(relative(1.15))
                                            .child(version_label.to_owned()),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(version_meta.to_owned()),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .mt_1()
                                            .child(
                                                Icon::new(IconName::FolderOpen)
                                                    .xsmall()
                                                    .text_color(cx.theme().muted_foreground),
                                            )
                                            .child(
                                                div()
                                                    .max_w(rems(23.75))
                                                    .truncate()
                                                    .text_xs()
                                                    .font_family(
                                                        cx.theme().mono_font_family.clone(),
                                                    )
                                                    .text_color(
                                                        cx.theme().muted_foreground.opacity(0.75),
                                                    )
                                                    .child(selected.map_or_else(
                                                        || {
                                                            self.config
                                                                .minecraft_root
                                                                .display()
                                                                .to_string()
                                                        },
                                                        |version| {
                                                            version.game_dir.display().to_string()
                                                        },
                                                    )),
                                            ),
                                    )
                                    .when(!has_account, |this| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().warning)
                                                .child("启动游戏前需要添加一个离线账户。"),
                                        )
                                    }),
                            ),
                    )
                    .child(
                        v_flex()
                            .w(rems(17.5))
                            .flex_shrink_0()
                            .gap_2()
                            .rounded(cx.theme().radius)
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().background.opacity(0.72))
                            .p_4()
                            .child(
                                div()
                                    .text_xs()
                                    .font_medium()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("启动配置"),
                            )
                            .child(
                                v_flex()
                                    .gap_1()
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
                                            .menu_width(layout::rem_px(cx, layout::MENU_WIDTH))
                                            .menu_max_h(layout::rem_px(cx, layout::MENU_MAX_HEIGHT))
                                            .cleanable(false)
                                            .w_full(),
                                    ),
                            )
                            .when(selected.is_some(), |this| {
                                this.child(
                                    Button::new("home-launch")
                                        .primary()
                                        .large()
                                        .icon(IconName::Play)
                                        .label(if self.launch_resolving {
                                            "正在验证账户…"
                                        } else {
                                            "启动游戏"
                                        })
                                        .w_full()
                                        .disabled(
                                            !can_start
                                                || !has_account
                                                || selected.is_none()
                                                || self.launch_resolving,
                                        )
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            if let Some(version) = this.selected_version() {
                                                let id = version.id.as_str().to_owned();
                                                this.start_launch(id, window, cx);
                                            }
                                        })),
                                )
                            })
                            .when(!has_account, |this| {
                                this.child(
                                    Button::new("home-account")
                                        .outline()
                                        .icon(IconName::CircleUser)
                                        .label("添加账户")
                                        .w_full()
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.open_accounts_dialog(window, cx)
                                        })),
                                )
                            })
                            .when(selected.is_none(), |this| {
                                this.child(
                                    Button::new("home-first-install")
                                        .primary()
                                        .large()
                                        .icon(IconName::Download)
                                        .label("安装第一个实例")
                                        .w_full()
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.navigate(Page::Downloads, window, cx)
                                        })),
                                )
                            }),
                    ),
            )
    }

    fn render_quick_connect(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let total = self.config.servers.len();
        let servers = self
            .config
            .servers
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>();
        let (can_join, join_tooltip) = self.server_join_ready();
        let selected_name = self
            .selected_version()
            .map(|version| version.display_name().to_owned());
        let subtitle = match selected_name {
            Some(name) => format!("使用「{name}」直接进入"),
            None => "选择实例后可直接进入".to_owned(),
        };
        let manage_label = if total > 3 {
            format!("查看全部 {total} 个")
        } else {
            "管理服务器".to_owned()
        };

        let mut tiles = Vec::new();
        for server in &servers {
            let view = self.servers.statuses.get(&server.id);
            let loading = view.is_none_or(|view| view.loading);
            let status = view.and_then(|view| view.status.as_ref());
            let icon_path = view.and_then(|view| view.icon_path.as_deref());
            let motd = status
                .map(|status| status.motd.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|motd| !motd.is_empty());
            let id = server.id.clone();
            let name = server.name.clone();
            tiles.push(
                h_flex()
                    .flex_basis(rems(0.))
                    .flex_grow(1.0)
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background.opacity(0.55))
                    .p_3()
                    .child(server_tile(icon_path, layout::AVATAR_MEDIUM, cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(div().text_sm().font_medium().truncate().child(name.clone()))
                            .child(server_status_line(loading, status, cx))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .truncate()
                                    .child(server.address.clone()),
                            )
                            .child(
                                div()
                                    .min_h(px(16.))
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .truncate()
                                    .child(motd.unwrap_or_default()),
                            ),
                    )
                    .child(
                        v_flex()
                            .w(rems(6.5))
                            .flex_shrink_0()
                            .self_stretch()
                            .items_end()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .w_full()
                                    .min_h(px(16.))
                                    .flex()
                                    .items_center()
                                    .justify_end()
                                    .when_some(server_players(status, cx), |this, players| {
                                        this.child(players)
                                    }),
                            )
                            .child(
                                Button::new(format!("home-server-join-{id}"))
                                    .outline()
                                    .small()
                                    .icon(IconName::Play)
                                    .label("进入")
                                    .w_full()
                                    .disabled(!can_join)
                                    .tooltip(join_tooltip)
                                    .accessibility_label(format!("进入 {name}"))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.join_server(id.clone(), window, cx)
                                    })),
                            ),
                    )
                    .into_any_element(),
            );
        }

        card(cx)
            .child(
                h_flex()
                    .w_full()
                    .items_start()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(section_title("快速连接", &subtitle, cx)),
                    )
                    .when(total > 0, |this| {
                        this.child(
                            Button::new("home-servers-manage")
                                .ghost()
                                .small()
                                .label(manage_label)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.navigate(Page::Servers, window, cx)
                                })),
                        )
                    }),
            )
            .when(servers.is_empty(), |this| {
                this.child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_3()
                        .rounded(cx.theme().radius)
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().background.opacity(0.55))
                        .p_3()
                        .child(
                            div()
                                .size_9()
                                .flex_shrink_0()
                                .rounded(cx.theme().radius)
                                .bg(cx.theme().primary.opacity(0.12))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(Icon::new(IconName::Network).text_color(cx.theme().primary)),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_0p5()
                                .child(div().text_sm().font_medium().child("还没有收藏的服务器"))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("添加服务器或收藏精选条目后，可以从首页直接进入。"),
                                ),
                        )
                        .child(
                            Button::new("home-server-add")
                                .primary()
                                .small()
                                .icon(IconName::Plus)
                                .label("添加服务器")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_server_dialog(None, window, cx)
                                })),
                        )
                        .child(
                            Button::new("home-server-discover")
                                .ghost()
                                .small()
                                .label("浏览精选")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.navigate(Page::Servers, window, cx);
                                    this.select_server_tab(ServerTab::Discover, window, cx);
                                })),
                        ),
                )
            })
            .when(!servers.is_empty(), |this| {
                this.child(h_flex().w_full().items_stretch().gap_3().children(tiles))
            })
    }

    fn render_recent_versions(&self, can_start: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows = Vec::new();
        for version in self.installed_versions.iter().take(3) {
            rows.push(
                self.render_home_version_row(version, can_start, cx)
                    .into_any_element(),
            );
        }
        card(cx)
            .child(section_title("最近实例", "", cx))
            .when(self.installed_versions.is_empty(), |this| {
                this.child(
                    v_flex()
                        .w_full()
                        .py_6()
                        .items_center()
                        .gap_2()
                        .child(
                            Icon::new(IconName::Boxes)
                                .with_size(Size::Large)
                                .text_color(cx.theme().muted_foreground.opacity(0.6)),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("还没有已安装实例"),
                        ),
                )
            })
            .children(rows)
    }

    fn render_home_version_row(
        &self,
        version: &InstalledVersion,
        can_start: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = version.id.as_str().to_owned();
        let name = version.display_name().to_owned();
        let meta = if version.display_name() != version.id.as_str() {
            format!(
                "{} · Minecraft {}",
                short_date(version.release_time.as_deref()),
                version.minecraft_version
            )
        } else {
            short_date(version.release_time.as_deref())
        };
        h_flex()
            .w_full()
            .gap_3()
            .rounded(cx.theme().radius)
            .px_2()
            .py_2()
            .child(version_tile(IconName::Box, cx.theme().primary, cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(div().text_sm().font_medium().truncate().child(name))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(meta),
                    ),
            )
            .child(type_tag(version.version_type.as_deref()))
            .child(if version.is_isolated() {
                Tag::secondary().child("隔离").small()
            } else {
                Tag::new().outline().child("共享").small()
            })
            .child(
                Button::new(format!("home-row-play-{id}"))
                    .outline()
                    .small()
                    .icon(IconName::Play)
                    .label("启动")
                    .disabled(!can_start)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.start_launch(id.clone(), window, cx)
                    })),
            )
    }

    fn render_quick_actions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        card(cx)
            .child(section_title("快速操作", "", cx))
            .child(
                Button::new("quick-install")
                    .outline()
                    .icon(IconName::Download)
                    .label("安装新实例")
                    .w_full()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.navigate(Page::Downloads, window, cx)
                    })),
            )
            .child(
                Button::new("quick-folder")
                    .outline()
                    .icon(IconName::FolderOpen)
                    .label("打开游戏目录")
                    .w_full()
                    .on_click(cx.listener(|this, _, window, cx| this.open_game_folder(window, cx))),
            )
            .child(
                v_flex()
                    .w_full()
                    .gap_1()
                    .pt_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Java"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_family(cx.theme().mono_font_family.clone())
                            .truncate()
                            .child(self.config.java_store.display().to_string()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if self.config.auto_install_java {
                                "Java 自动获取已开启"
                            } else {
                                "Java 自动获取已关闭"
                            }),
                    ),
            )
    }
}

fn hero_tile(has_version: bool, cx: &App) -> impl IntoElement {
    let icon = if has_version {
        IconName::Box
    } else {
        IconName::Download
    };
    div()
        .size(layout::HERO_TILE)
        .flex_shrink_0()
        .rounded(cx.theme().radius_4xl())
        .bg(cx.theme().primary.opacity(0.14))
        .border_1()
        .border_color(cx.theme().primary.opacity(0.18))
        .flex()
        .items_center()
        .justify_center()
        .child(
            Icon::new(icon)
                .with_size(Size::Large)
                .text_color(cx.theme().primary),
        )
}

fn error_banner(title: &str, message: &str, cx: &App) -> impl IntoElement {
    h_flex()
        .w_full()
        .gap_3()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().danger.opacity(0.35))
        .bg(cx.theme().danger.opacity(0.08))
        .p_3()
        .child(
            Icon::new(IconName::TriangleAlert)
                .small()
                .text_color(cx.theme().danger),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_medium()
                        .text_color(cx.theme().danger)
                        .child(title.to_owned()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(message.to_owned()),
                ),
        )
}
