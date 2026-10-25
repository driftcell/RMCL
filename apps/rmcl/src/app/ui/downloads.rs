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

use super::super::{
    LauncherApp, Page, VersionFilter, short_date, version_tile, version_type_label,
};
use super::common::{card, empty_state, page_header, type_tag};

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
                            .tooltip("刷新 Mojang 版本列表")
                            .accessibility_label("刷新 Mojang 版本列表")
                            .disabled(refreshing)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.load_catalog(true, window, cx)
                            })),
                    )
                    .child(
                        Button::new("downloads-installed")
                            .outline()
                            .icon(IconName::Boxes)
                            .label("管理已安装版本")
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
                                    .w(px(320.)),
                            )
                            .child(
                                TabBar::new("downloads-filter")
                                    .segmented()
                                    .selected_index(self.downloads_filter.index())
                                    .on_click(cx.listener(|this, index: &usize, _, cx| {
                                        this.downloads_filter = VersionFilter::from_index(*index);
                                        this.downloads_scroll
                                            .scroll_to_item(0, ScrollStrategy::Top);
                                        cx.notify();
                                    }))
                                    .children(
                                        VersionFilter::ALL
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
                                        Some(manifest) => {
                                            format!(
                                                "Mojang 官方 · {} 个版本",
                                                manifest.versions.len()
                                            )
                                        }
                                        None => "Mojang 官方".to_owned(),
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
                                .child("正在获取 Mojang 版本列表…"),
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
                    "无法加载版本列表",
                    error,
                    Button::new("downloads-retry")
                        .primary()
                        .icon(IconName::RefreshCw)
                        .label("重试")
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
                    "版本列表尚未加载",
                    "刷新后从 Mojang 官方源获取可安装的 Vanilla 版本。",
                    Button::new("downloads-load")
                        .primary()
                        .icon(IconName::RefreshCw)
                        .label("加载版本列表")
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
                    "没有匹配的版本",
                    "试试更换搜索词或筛选条件。",
                    Button::new("downloads-clear")
                        .outline()
                        .label("查看全部版本")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.downloads_filter = VersionFilter::All;
                            cx.notify();
                        })),
                    cx,
                ))
                .into_any_element();
        }

        let count = versions.len();
        let sizes = Rc::new(vec![size(px(0.), px(64.)); count]);
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
                    .h(px(38.))
                    .px_4()
                    .gap_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().list_head)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(div().flex_1().child("版本"))
                    .child(div().w(px(112.)).child("发布时间"))
                    .child(div().w(px(86.)).child("状态"))
                    .child(div().w(px(104.)).child("操作")),
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
            .h(px(64.))
            .px_4()
            .gap_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .hover(|style| style.bg(cx.theme().list_hover))
            .child(version_tile(icon, cx.theme().primary))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().text_sm().font_semibold().truncate().child(id.clone()))
                            .child(type_tag(version.version_type.as_deref()))
                            .when(latest, |this| {
                                this.child(Tag::primary().child("最新正式版").small())
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(version_type_label(version.version_type.as_deref())),
                    ),
            )
            .child(
                div()
                    .w(px(112.))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(date),
            )
            .child(div().w(px(86.)).when(installed, |this| {
                this.child(Tag::success().child("已安装").small())
            }))
            .child(
                div().w(px(104.)).child(
                    Button::new(format!("downloads-install-{id}"))
                        .outline()
                        .small()
                        .icon(IconName::Download)
                        .label(if installed { "重新安装" } else { "安装" })
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
