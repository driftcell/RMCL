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
use rmcl_app::InstalledVersion;

use super::super::{LauncherApp, Page, VersionFilter, short_date, version_tile};
use super::common::{card, display_path, empty_state, page_header, section_title, type_tag};
use crate::i18n::tr;

impl LauncherApp {
    pub fn render_versions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let versions = self.filtered_installed_versions(cx);
        let can_start = self.can_start_operation();
        let shared_count = self
            .installed_versions
            .iter()
            .filter(|version| !version.is_isolated() && version.manifest_error.is_none())
            .count();
        let mut rows = Vec::new();
        for version in &versions {
            rows.push(
                self.render_version_row(version, can_start, cx)
                    .into_any_element(),
            );
        }

        v_flex()
            .flex_1()
            .min_h_0()
            .p_6()
            .gap_6()
            .child(page_header(
                Page::Versions,
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("versions-refresh")
                            .ghost()
                            .icon(IconName::RefreshCw)
                            .tooltip(tr("app.versions.rescan"))
                            .accessibility_label(tr("app.versions.rescan"))
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.refresh_versions(window, cx)
                                }),
                            ),
                    )
                    .child(
                        Button::new("versions-create")
                            .primary()
                            .icon(IconName::PackagePlus)
                            .label(tr("app.versions.create"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_create_instance(window, cx)
                            })),
                    ),
                cx,
            ))
            .child(
                v_flex()
                    .id("versions-body")
                    .flex_1()
                    .min_h_0()
                    .gap_4()
                    .overflow_y_scrollbar()
                    .child(
                        h_flex()
                            .gap_3()
                            .child(
                                Input::new(&self.versions_query)
                                    .prefix(Icon::new(IconName::Search).small())
                                    .cleanable(true)
                                    .w(rems(18.75)),
                            )
                            .child(
                                TabBar::new("versions-filter")
                                    .segmented()
                                    .selected_index(self.versions_filter.index())
                                    .on_click(cx.listener(|this, index: &usize, _, cx| {
                                        this.versions_filter = VersionFilter::from_index(*index);
                                        cx.notify();
                                    }))
                                    .children(
                                        VersionFilter::ALL
                                            .into_iter()
                                            .map(|filter| Tab::new().label(filter.label())),
                                    ),
                            ),
                    )
                    .when(shared_count > 0, |this| {
                        this.child(
                            h_flex()
                                .w_full()
                                .gap_3()
                                .rounded(cx.theme().radius)
                                .border_1()
                                .border_color(cx.theme().primary.opacity(0.25))
                                .bg(cx.theme().primary.opacity(0.06))
                                .p_3()
                                .child(
                                    Icon::new(IconName::Info)
                                        .small()
                                        .text_color(cx.theme().primary),
                                )
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap_0p5()
                                        .child(
                                            div().text_sm().font_medium().child(
                                                rust_i18n::t!(
                                                    "app.versions.shared_banner",
                                                    count = shared_count
                                                )
                                                .to_string(),
                                            ),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(tr("app.versions.shared_banner_hint")),
                                        ),
                                ),
                        )
                    })
                    .when_some(self.versions_error.clone(), |this, error| {
                        this.child(
                            div()
                                .rounded(cx.theme().radius)
                                .border_1()
                                .border_color(cx.theme().danger.opacity(0.35))
                                .bg(cx.theme().danger.opacity(0.08))
                                .p_3()
                                .text_sm()
                                .text_color(cx.theme().danger)
                                .child(error),
                        )
                    })
                    .child(if versions.is_empty() {
                        card(cx)
                            .child(empty_state(
                                IconName::Boxes,
                                if self.installed_versions.is_empty() {
                                    tr("app.versions.empty_title")
                                } else {
                                    tr("app.versions.no_match_title")
                                },
                                if self.installed_versions.is_empty() {
                                    tr("app.versions.empty_description")
                                } else {
                                    tr("app.common.try_clear_search")
                                },
                                Button::new("versions-empty-install")
                                    .primary()
                                    .icon(IconName::Download)
                                    .label(if self.installed_versions.is_empty() {
                                        tr("app.home.install_first")
                                    } else {
                                        tr("app.versions.browse_all")
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.navigate(Page::Downloads, window, cx)
                                    })),
                                cx,
                            ))
                            .into_any_element()
                    } else {
                        card(cx)
                            .child(section_title(
                                tr("app.versions.installed_title"),
                                rust_i18n::t!(
                                    "app.versions.installed_count",
                                    count = versions.len()
                                )
                                .to_string(),
                                cx,
                            ))
                            .children(rows)
                            .into_any_element()
                    }),
            )
    }

    fn render_version_row(
        &self,
        version: &InstalledVersion,
        can_start: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = version.id.as_str().to_owned();
        let display_name = version.display_name().to_owned();
        let path = display_path(&version.game_dir);
        let meta = if version.minecraft_version != id {
            format!(
                "Minecraft {} · {}",
                version.minecraft_version,
                short_date(version.release_time.as_deref())
            )
        } else {
            short_date(version.release_time.as_deref())
        };
        let loader = version.loader.clone();
        let manifest_error = version.manifest_error.clone();
        let manifest_invalid = manifest_error.is_some();
        let isolated = version.is_isolated();
        let play_id = id.clone();
        let manage_id = id.clone();
        let repair_id = id.clone();
        let folder_id = id.clone();
        let isolate_id = id.clone();
        let delete_id = id.clone();
        h_flex()
            .w_full()
            .gap_3()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background.opacity(0.35))
            .p_3()
            .child(version_tile(IconName::Box, cx.theme().primary, cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .text_base()
                                    .font_semibold()
                                    .truncate()
                                    .child(display_name),
                            )
                            .when(version.display_name() != version.id.as_str(), |this| {
                                this.child(Tag::new().outline().child(id.clone()).small())
                            })
                            .child(type_tag(version.version_type.as_deref()))
                            .when_some(loader, |this, loader| {
                                this.child(
                                    Tag::new()
                                        .outline()
                                        .child(format!(
                                            "{} {}",
                                            loader.kind.label(),
                                            loader.version
                                        ))
                                        .small(),
                                )
                            })
                            .when(!isolated, |this| {
                                this.child(
                                    Tag::new().outline().child(tr("app.common.shared")).small(),
                                )
                            })
                            .when(manifest_error.is_some(), |this| {
                                this.child(
                                    Tag::danger()
                                        .child(tr("app.versions.manifest_invalid"))
                                        .small(),
                                )
                            })
                            .when(!version.client_jar_present, |this| {
                                this.child(
                                    Tag::warning().child(tr("app.versions.incomplete")).small(),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(meta),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_color(cx.theme().muted_foreground.opacity(0.75))
                            .truncate()
                            .child(rust_i18n::t!("app.versions.game_dir", path = path).to_string()),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new(format!("versions-play-{play_id}"))
                            .outline()
                            .small()
                            .icon(IconName::Play)
                            .label(tr("app.home.launch"))
                            .disabled(!can_start)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.start_launch(play_id.clone(), window, cx)
                            })),
                    )
                    .child(
                        Button::new(format!("versions-manage-{manage_id}"))
                            .ghost()
                            .small()
                            .icon(IconName::Package)
                            .tooltip(tr("app.versions.manage_content"))
                            .accessibility_label(tr("app.versions.manage_content"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_instance_detail(manage_id.clone(), window, cx)
                            })),
                    )
                    .child(
                        Button::new(format!("versions-repair-{repair_id}"))
                            .ghost()
                            .small()
                            .icon(IconName::Wrench)
                            .tooltip(tr("app.versions.repair"))
                            .accessibility_label(tr("app.versions.repair"))
                            .disabled(!can_start)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.start_repair(repair_id.clone(), window, cx)
                            })),
                    )
                    .when(!isolated, |this| {
                        this.child(
                            Button::new(format!("versions-isolate-{isolate_id}"))
                                .ghost()
                                .small()
                                .icon(IconName::FolderLock)
                                .tooltip(tr("app.versions.enable_isolation"))
                                .accessibility_label(tr("app.versions.enable_isolation"))
                                .disabled(!can_start || manifest_invalid)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.confirm_enable_isolation(isolate_id.clone(), window, cx)
                                })),
                        )
                    })
                    .child(
                        Button::new(format!("versions-folder-{folder_id}"))
                            .ghost()
                            .small()
                            .icon(IconName::FolderOpen)
                            .tooltip(tr("app.home.open_game_folder"))
                            .accessibility_label(tr("app.home.open_game_folder"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_instance_folder(folder_id.as_str(), window, cx)
                            })),
                    )
                    .child(
                        Button::new(format!("versions-delete-{delete_id}"))
                            .ghost()
                            .small()
                            .icon(IconName::Trash)
                            .tooltip(tr("app.versions.delete"))
                            .accessibility_label(tr("app.versions.delete"))
                            .disabled(!can_start)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm_delete_version(delete_id.clone(), window, cx)
                            })),
                    ),
            )
    }
}
