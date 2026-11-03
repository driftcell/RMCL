use std::rc::Rc;

use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon, Sizable, Size, StyledExt,
        button::{Button, ButtonVariants},
        h_flex,
        input::Input,
        spinner::Spinner,
        tab::{Tab, TabBar},
        tag::Tag,
        v_flex, v_virtual_list,
    },
    prelude::FluentBuilder,
    *,
};
use rmcl_app::VersionSummary;

use super::super::{LauncherApp, Page, VersionFilter, short_date, version_tile};
use super::common::{card, empty_state, page_header, type_tag};
use super::layout;
use crate::i18n::tr;

impl LauncherApp {
    pub fn render_downloads(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let refreshing = self.catalog.loading;
        v_flex()
            .flex_1()
            .min_h_0()
            .p_6()
            .gap_6()
            .child(page_header(
                Page::Downloads,
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("downloads-refresh")
                            .ghost()
                            .icon(IconName::RefreshCw)
                            .tooltip(tr("app.downloads.refresh"))
                            .accessibility_label(tr("app.downloads.refresh"))
                            .disabled(refreshing)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.load_catalog(true, window, cx)
                            })),
                    )
                    .child(
                        Button::new("downloads-installed")
                            .outline()
                            .icon(IconName::Boxes)
                            .label(tr("app.downloads.manage_installed"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.navigate(Page::Versions, window, cx)
                            })),
                    ),
                cx,
            ))
            .child(
                v_flex()
                    .id("downloads-body")
                    .flex_1()
                    .min_h_0()
                    .gap_4()
                    .child(
                        h_flex()
                            .gap_3()
                            .child(
                                Input::new(&self.downloads_query)
                                    .prefix(Icon::new(IconName::Search).small())
                                    .cleanable(true)
                                    .w_80(),
                            )
                            .child(
                                TabBar::new("downloads-filter")
                                    .segmented()
                                    .selected_index(self.downloads_filter.download_index())
                                    .on_click(cx.listener(|this, index: &usize, _, cx| {
                                        this.downloads_filter =
                                            VersionFilter::from_download_index(*index);
                                        this.downloads_scroll
                                            .scroll_to_item(0, ScrollStrategy::Top);
                                        cx.notify();
                                    }))
                                    .children(
                                        VersionFilter::DOWNLOADS
                                            .into_iter()
                                            .map(|filter| Tab::new().label(filter.label())),
                                    ),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(match &self.catalog.manifest {
                                        Some(manifest) => rust_i18n::t!(
                                            "app.downloads.official_count",
                                            count = manifest.versions.len()
                                        )
                                        .to_string(),
                                        None => tr("app.downloads.official"),
                                    }),
                            ),
                    )
                    .child(self.render_catalog_body(cx)),
            )
    }

    fn render_catalog_body(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.catalog.loading && self.catalog.manifest.is_none() {
            return card(cx)
                .child(
                    v_flex()
                        .w_full()
                        .items_center()
                        .justify_center()
                        .gap_3()
                        .py_16()
                        .child(
                            Spinner::new()
                                .with_size(Size::Large)
                                .color(cx.theme().primary),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr("app.downloads.loading")),
                        ),
                )
                .into_any_element();
        }
        if let Some(error) = &self.catalog.error
            && self.catalog.manifest.is_none()
        {
            return card(cx)
                .child(empty_state(
                    IconName::CircleAlert,
                    tr("app.downloads.load_failed"),
                    error,
                    Button::new("downloads-retry")
                        .primary()
                        .icon(IconName::RefreshCw)
                        .label(tr("app.common.retry"))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.load_catalog(true, window, cx)),
                        ),
                    cx,
                ))
                .into_any_element();
        }
        if self.catalog.manifest.is_none() {
            return card(cx)
                .child(empty_state(
                    IconName::Download,
                    tr("app.downloads.not_loaded"),
                    tr("app.downloads.not_loaded_description"),
                    Button::new("downloads-load")
                        .primary()
                        .icon(IconName::RefreshCw)
                        .label(tr("app.downloads.load_list"))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.load_catalog(false, window, cx)),
                        ),
                    cx,
                ))
                .into_any_element();
        }

        let versions = Rc::new(self.filtered_remote_versions(cx));
        if versions.is_empty() {
            return card(cx)
                .child(empty_state(
                    IconName::Search,
                    tr("app.downloads.no_match"),
                    tr("app.downloads.no_match_description"),
                    Button::new("downloads-clear")
                        .outline()
                        .label(tr("app.downloads.show_releases"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.downloads_filter = VersionFilter::Release;
                            this.downloads_query
                                .update(cx, |state, cx| state.set_value("", window, cx));
                            this.downloads_scroll.scroll_to_item(0, ScrollStrategy::Top);
                            cx.notify();
                        })),
                    cx,
                ))
                .into_any_element();
        }

        let count = versions.len();
        let row_height = layout::rem_px(cx, layout::TABLE_ROW_HEIGHT);
        let sizes = Rc::new(vec![size(px(0.), row_height); count]);
        let entity = cx.entity();
        let list = v_virtual_list(entity, "downloads-list", sizes, {
            let versions = versions.clone();
            move |this, range, window, cx| {
                range
                    .map(|index| {
                        this.render_remote_row(&versions[index], window, cx)
                            .into_any_element()
                    })
                    .collect::<Vec<_>>()
            }
        })
        .track_scroll(&self.downloads_scroll);

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
                    .gap_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().list_head)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(div().flex_1().child(tr("app.downloads.column_version")))
                    .child(
                        div()
                            .w(layout::DOWNLOAD_DATE_COLUMN)
                            .child(tr("app.downloads.column_released")),
                    )
                    .child(
                        div()
                            .w(layout::DOWNLOAD_STATUS_COLUMN)
                            .child(tr("app.downloads.column_status")),
                    )
                    .child(
                        div()
                            .w(layout::DOWNLOAD_ACTION_COLUMN)
                            .child(tr("app.downloads.column_action")),
                    ),
            )
            .child(list.flex_1().min_h_0())
            .into_any_element()
    }

    fn render_remote_row(
        &mut self,
        version: &VersionSummary,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let installed = self.is_installed(&version.id);
        let latest = self.latest_release() == Some(version.id.as_str());
        let id = version.id.clone();
        let icon = version_icon(version.version_type.as_deref());
        let date = short_date(version.release_time.as_deref());
        let can_start = self.log_state.can_start();
        h_flex()
            .w_full()
            .h(layout::TABLE_ROW_HEIGHT)
            .px_4()
            .gap_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(version_tile(icon, cx.theme().primary, cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(div().text_sm().font_semibold().truncate().child(id.clone()))
                    .child(type_tag(version.version_type.as_deref()))
                    .when(latest, |this| {
                        this.child(
                            Tag::primary()
                                .child(tr("app.downloads.latest_release"))
                                .small(),
                        )
                    }),
            )
            .child(
                div()
                    .w(layout::DOWNLOAD_DATE_COLUMN)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(date),
            )
            .child(
                div()
                    .w(layout::DOWNLOAD_STATUS_COLUMN)
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(installed, |this| {
                        this.child(
                            Icon::new(IconName::CircleCheck)
                                .with_size(Size::Medium)
                                .text_color(cx.theme().success),
                        )
                    }),
            )
            .child(
                div().w(layout::DOWNLOAD_ACTION_COLUMN).child(
                    Button::new(format!("downloads-install-{id}"))
                        .outline()
                        .small()
                        .icon(IconName::Download)
                        .label(if installed {
                            tr("app.downloads.reinstall")
                        } else {
                            tr("app.common.install")
                        })
                        .disabled(!can_start)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(version) = this
                                .catalog
                                .manifest
                                .as_ref()
                                .and_then(|manifest| manifest.versions.iter().find(|v| v.id == id))
                                .cloned()
                            {
                                this.open_install_dialog(version, window, cx);
                            }
                        })),
                ),
            )
    }
}

fn version_icon(version_type: Option<&str>) -> IconName {
    match version_type {
        Some("release") => IconName::Package,
        Some("snapshot") => IconName::FlaskConical,
        Some("old_beta") | Some("old_alpha") => IconName::Archive,
        _ => IconName::Package,
    }
}
