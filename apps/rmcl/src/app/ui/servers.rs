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
use rmcl_app::{FeaturedServer, ServerStatus, featured_servers};
use rmcl_store::StoredServer;

use super::super::{LauncherApp, Page, ServerTab};
use super::common::{card, empty_state, error_banner, page_header, server_tile};
use super::layout;
use crate::i18n::tr;

impl LauncherApp {
    pub fn render_servers(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_1()
            .min_h_0()
            .p_6()
            .gap_5()
            .child(page_header(Page::Servers, div(), cx))
            .child(
                h_flex()
                    .w_full()
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
                    .child(div().flex_1())
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(tr("app.servers.launch_target")),
                            )
                            .child(
                                div().w(rems(13.5)).child(
                                    Select::new(&self.version_select)
                                        .placeholder(tr("app.home.select_instance"))
                                        .accessibility_label(tr(
                                            "app.servers.launch_target_instance",
                                        ))
                                        .menu_width(layout::rem_px(cx, layout::MENU_WIDTH))
                                        .menu_max_h(layout::rem_px(cx, layout::MENU_MAX_HEIGHT))
                                        .cleanable(false)
                                        .w_full(),
                                ),
                            ),
                    ),
            )
            .child(match self.server_tab {
                ServerTab::Favorites => self.render_server_favorites(cx).into_any_element(),
                ServerTab::Discover => self.render_server_discover(cx),
            })
    }

    fn add_server_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Button::new("servers-add")
            .primary()
            .icon(IconName::Plus)
            .label(tr("app.servers.add"))
            .on_click(cx.listener(|this, _, window, cx| this.open_server_dialog(None, window, cx)))
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
        let (can_join, join_tooltip) = self.server_join_ready();

        let mut rows = Vec::new();
        for server in &servers {
            rows.push(self.render_server_row(server, can_join, &join_tooltip, cx));
        }

        v_flex().flex_1().min_h_0().gap_5().child(
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
                                .w(rems(22.5))
                                .flex_shrink_0()
                                .child(Input::new(&self.server_query).w_full()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(
                                    rust_i18n::t!("app.servers.total", count = total).to_string(),
                                ),
                        )
                        .child(div().flex_1())
                        .child(self.add_server_button(cx)),
                )
                .when(servers.is_empty(), |this| {
                    this.child(empty_state(
                        IconName::Network,
                        if total == 0 {
                            tr("app.servers.empty_title")
                        } else {
                            tr("app.servers.no_match_title")
                        },
                        if total == 0 {
                            tr("app.servers.empty_description")
                        } else {
                            tr("app.common.try_clear_search")
                        },
                        Button::new("servers-empty-add")
                            .primary()
                            .icon(IconName::Plus)
                            .label(tr("app.servers.add"))
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
                    .child(error_banner(
                        tr("app.servers.featured_load_failed"),
                        error,
                        cx,
                    ))
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
        let (can_join, join_tooltip) = self.server_join_ready();

        let mut rows = Vec::new();
        for server in &servers {
            rows.push(self.render_featured_server_row(server, can_join, &join_tooltip, cx));
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
                            .w(rems(22.5))
                            .flex_shrink_0()
                            .child(Input::new(&self.discover_query).w_full()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                rust_i18n::t!(
                                    "app.servers.shown",
                                    shown = servers.len(),
                                    total = total
                                )
                                .to_string(),
                            ),
                    ),
            )
            .when(servers.is_empty(), |this| {
                this.child(empty_state(
                    IconName::Globe,
                    tr("app.servers.featured_no_match"),
                    tr("app.common.try_clear_search"),
                    div(),
                    cx,
                ))
            })
            .children(rows)
            .into_any_element()
    }

    fn render_featured_server_row(
        &self,
        server: &FeaturedServer,
        can_join: bool,
        join_tooltip: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = self.servers.featured.statuses.get(&server.id);
        let loading = view.is_none_or(|view| view.loading);
        let status = view.and_then(|view| view.status.as_ref());
        let icon_path = view.and_then(|view| view.icon_path.as_deref());
        let favorited = self.featured_server_is_favorite(&server.address);
        let favorite_tooltip = if favorited {
            tr("app.servers.unfavorite_tooltip")
        } else {
            tr("app.servers.favorite_tooltip")
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
            .tooltip(favorite_tooltip.clone())
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
                .link()
                .small()
                .icon(IconName::ExternalLink)
                .tooltip(tr("app.servers.open_website"))
                .accessibility_label(tr("app.servers.open_website"))
                .on_click(move |_, _, cx| cx.open_url(&url))
        });
        let actionable = can_join && !server.requires_mods;
        let action_label = if server.requires_mods {
            tr("app.servers.requires_modpack")
        } else {
            tr("app.servers.join")
        };
        let action_tooltip = if server.requires_mods {
            tr("app.servers.requires_modpack_tooltip")
        } else {
            join_tooltip.to_owned()
        };

        let mut tags = Vec::new();
        if server.requires_mods {
            tags.push(
                Tag::warning()
                    .small()
                    .child(tr("app.servers.requires_mods_tag")),
            );
        }
        for tag in &server.tags {
            tags.push(Tag::secondary().small().child(tag.clone()));
        }
        let version_note = server
            .version_note
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| tr("app.servers.version_unknown"));

        card(cx)
            .gap_3()
            .child(
                h_flex()
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
                                        server
                                            .version
                                            .as_deref()
                                            .map(str::to_owned)
                                            .unwrap_or_else(|| {
                                                tr("app.servers.select_modpack_version")
                                            })
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
                            .w_48()
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
                                            .icon(IconName::Play)
                                            .label(action_label)
                                            .disabled(!actionable)
                                            .tooltip(action_tooltip)
                                            .on_click(cx.listener({
                                                let server = server.clone();
                                                move |this, _, window, cx| {
                                                    this.join_featured_server(
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
        let loading = view.is_none_or(|view| view.loading);
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
                                    .label(tr("app.servers.join"))
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
                                    .tooltip(tr("app.servers.edit"))
                                    .accessibility_label(tr("app.servers.edit"))
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
                                    .tooltip(tr("app.servers.delete"))
                                    .accessibility_label(tr("app.servers.delete"))
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

pub(super) fn server_players(status: Option<&ServerStatus>, cx: &App) -> Option<AnyElement> {
    let status = status?;
    let players = match (status.online_players, status.max_players) {
        (Some(online), Some(max)) => {
            rust_i18n::t!("app.servers.players_of", online = online, max = max).to_string()
        }
        (Some(online), None) => rust_i18n::t!("app.servers.players", online = online).to_string(),
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

pub(super) fn server_status_line(
    loading: bool,
    status: Option<&ServerStatus>,
    cx: &App,
) -> AnyElement {
    if loading && status.is_none() {
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
                    .child(tr("app.servers.querying")),
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
                    .child(tr("app.servers.unreachable")),
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
