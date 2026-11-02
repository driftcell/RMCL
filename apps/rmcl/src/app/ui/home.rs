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
use super::common::{card, display_path, page_header, section_title, server_tile, type_tag};
use super::layout;
use super::servers::{server_players, server_status_line};
use crate::i18n::tr;

impl LauncherApp {
    pub fn render_home(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.selected_version();
        let version_label = selected
            .map(|version| version.display_name().to_owned())
            .unwrap_or_else(|| tr("app.home.first_instance_ready"));
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
            .unwrap_or_else(|| tr("app.home.first_instance_hint"));
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
                        this.child(error_banner(tr("app.home.config_read_failed"), warning, cx))
                    })
                    .when_some(self.config_save_error.clone(), |this, error| {
                        this.child(error_banner(tr("app.home.config_save_failed"), error, cx))
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
                            .items_stretch()
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
                                                        tr("app.home.current_instance")
                                                    } else {
                                                        tr("app.home.not_installed")
                                                    })
                                                    .small(),
                                            )
                                            .when_some(selected, |this, version| {
                                                this.when(!version.is_isolated(), |this| {
                                                    this.child(
                                                        Tag::new()
                                                            .outline()
                                                            .child(tr("app.common.shared"))
                                                            .small(),
                                                    )
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
                                                            display_path(
                                                                &self.config.minecraft_root,
                                                            )
                                                        },
                                                        |version| display_path(&version.game_dir),
                                                    )),
                                            ),
                                    )
                                    .when(!has_account, |this| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().warning)
                                                .child(tr("app.home.account_required")),
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
                                    .child(tr("app.home.launch_config")),
                            )
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(tr("app.home.instance")),
                                    )
                                    .child(
                                        Select::new(&self.version_select)
                                            .placeholder(tr("app.home.select_instance"))
                                            .accessibility_label(tr("app.home.select_instance"))
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
                                            tr("app.home.verifying_account")
                                        } else {
                                            tr("app.home.launch_game")
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
                                        .label(tr("app.chrome.add_account"))
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
                                        .label(tr("app.home.install_first"))
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
        let manage_label = if total > 3 {
            rust_i18n::t!("app.home.view_all_servers", count = total).to_string()
        } else {
            tr("app.home.manage_servers")
        };

        let mut server_cards = Vec::new();
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
            server_cards.push(
                card(cx)
                    .gap_3()
                    .child(
                        h_flex()
                            .w_full()
                            .items_start()
                            .gap_3()
                            .child(server_tile(icon_path, layout::TILE_LARGE, cx))
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
                                                    .child(name.clone()),
                                            )
                                            .when_some(
                                                server_players(status, cx),
                                                |this, players| this.child(players),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .truncate()
                                            .child(server.address.clone()),
                                    )
                                    .when_some(motd, |this, motd| {
                                        this.child(
                                            div()
                                                .text_sm()
                                                .text_color(cx.theme().foreground)
                                                .truncate()
                                                .child(motd),
                                        )
                                    })
                                    .child(server_status_line(loading, status, cx)),
                            )
                            .child(
                                Button::new(format!("home-server-join-{id}"))
                                    .primary()
                                    .icon(IconName::Play)
                                    .label(tr("app.servers.join"))
                                    .flex_shrink_0()
                                    .disabled(!can_join)
                                    .tooltip(join_tooltip.clone())
                                    .accessibility_label(
                                        rust_i18n::t!("app.home.join_named", name = name)
                                            .to_string(),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.join_server(id.clone(), window, cx)
                                    })),
                            ),
                    )
                    .into_any_element(),
            );
        }

        v_flex()
            .w_full()
            .gap_3()
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(section_title(tr("app.home.quick_connect"), "", cx))
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
                    card(cx).child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .size_9()
                                    .flex_shrink_0()
                                    .rounded(cx.theme().radius)
                                    .bg(cx.theme().primary.opacity(0.12))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        Icon::new(IconName::Network).text_color(cx.theme().primary),
                                    ),
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
                                            .child(tr("app.home.no_favorite_servers")),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(tr("app.home.no_favorite_servers_hint")),
                                    ),
                            )
                            .child(
                                Button::new("home-server-add")
                                    .primary()
                                    .small()
                                    .icon(IconName::Plus)
                                    .label(tr("app.servers.add"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_server_dialog(None, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("home-server-discover")
                                    .ghost()
                                    .small()
                                    .label(tr("app.home.browse_featured"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.navigate(Page::Servers, window, cx);
                                        this.select_server_tab(ServerTab::Discover, window, cx);
                                    })),
                            ),
                    ),
                )
            })
            .children(server_cards)
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
            .h_full()
            .child(section_title(tr("app.home.recent_instances"), "", cx))
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
                                .child(tr("app.home.no_installed_instances")),
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
            .when(!version.is_isolated(), |this| {
                this.child(Tag::new().outline().child(tr("app.common.shared")).small())
            })
            .child(
                Button::new(format!("home-row-play-{id}"))
                    .outline()
                    .small()
                    .icon(IconName::Play)
                    .label(tr("app.home.launch"))
                    .disabled(!can_start)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.start_launch(id.clone(), window, cx)
                    })),
            )
    }

    fn render_quick_actions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        card(cx)
            .h_full()
            .child(section_title(tr("app.home.quick_actions"), "", cx))
            .child(
                Button::new("quick-install")
                    .outline()
                    .icon(IconName::Download)
                    .label(tr("app.home.install_new_instance"))
                    .w_full()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.navigate(Page::Downloads, window, cx)
                    })),
            )
            .child(
                Button::new("quick-folder")
                    .outline()
                    .icon(IconName::FolderOpen)
                    .label(tr("app.home.open_game_folder"))
                    .w_full()
                    .on_click(cx.listener(|this, _, window, cx| this.open_game_folder(window, cx))),
            )
            .child(div().flex_1().min_h_0())
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
                            .child(display_path(&self.config.java_store)),
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

fn error_banner(
    title: impl Into<SharedString>,
    message: impl Into<SharedString>,
    cx: &App,
) -> impl IntoElement {
    let title = title.into();
    let message = message.into();
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
                        .child(title.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(message.clone()),
                ),
        )
}
