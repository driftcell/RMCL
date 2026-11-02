use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon, Sizable, Size, StyledExt,
        button::{Button, ButtonVariants},
        h_flex,
        input::Input,
        scroll::ScrollableElement,
        spinner::Spinner,
        tab::{Tab, TabBar},
        tag::Tag,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use rmcl_app::{
    ContentInspection, ContentKind, InstalledVersion, LocalContent, ResourcePackCompatibility,
    ResourcePackMetadata, ResourcePackOrder, ShaderLoader, ShaderPackMetadata,
};

use super::super::{ContentTab, LauncherApp, Page, short_date, version_tile, version_type_label};
use super::common::{
    card, display_path, empty_state, empty_state_message, error_banner, format_size, mono,
    section_title, type_tag, warning_banner,
};
use crate::i18n::tr;

impl LauncherApp {
    pub fn render_instance(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(version) = self.selected_instance_version().cloned() else {
            return v_flex()
                .flex_1()
                .min_h_0()
                .p_6()
                .gap_4()
                .child(
                    Button::new("instance-back")
                        .ghost()
                        .icon(IconName::ArrowLeft)
                        .label(tr("app.content_browser.back_instances"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.navigate(Page::Versions, window, cx)
                        })),
                )
                .child(
                    card(cx).child(empty_state(
                        IconName::TriangleAlert,
                        tr("app.content_browser.instance_missing"),
                        tr("app.content_browser.instance_missing_description"),
                        Button::new("instance-missing-back")
                            .primary()
                            .label(tr("app.content_browser.back_instances"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.navigate(Page::Versions, window, cx)
                            })),
                        cx,
                    )),
                )
                .into_any_element();
        };
        let library = self.content_library.as_ref();
        let inspection = self.content_inspection.as_ref();
        let items = self.filtered_content(cx);
        let pending = self.content_operations_pending();
        let kind = self.content_tab.kind();
        let counts = ContentTab::ALL.map(|tab| {
            let kind = tab.kind();
            let total = library.map_or(0, |library| library.count(kind));
            let enabled = library.map_or(0, |library| library.enabled_count(kind));
            (total, enabled)
        });
        let warning = library.and_then(|library| library.warnings.first());
        let warning_count = library.map_or(0, |library| library.warnings.len());
        let warning_title =
            rust_i18n::t!("app.instance.scan_warnings", count = warning_count).to_string();
        let content_count = counts
            .get(self.content_tab.index())
            .copied()
            .unwrap_or_default();

        v_flex()
            .flex_1()
            .min_h_0()
            .p_6()
            .gap_5()
            .child(self.render_instance_header(&version, cx))
            .child(
                v_flex()
                    .id("instance-body")
                    .flex_1()
                    .min_h_0()
                    .gap_4()
                    .overflow_y_scrollbar()
                    .child(self.render_instance_summary(&version, counts, cx))
                    .when_some(self.content_error.clone(), |this, error: String| {
                        this.child(error_banner(tr("app.instance.scan_failed"), error, cx))
                    })
                    .when_some(
                        warning.map(|warning| warning.message()),
                        |this, message: String| {
                            this.child(warning_banner(&warning_title, &message, cx))
                        },
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .gap_3()
                            .items_center()
                            .child(
                                TabBar::new("instance-content-tabs")
                                    .segmented()
                                    .selected_index(self.content_tab.index())
                                    .on_click(cx.listener(|this, index: &usize, _, cx| {
                                        this.content_tab = ContentTab::from_index(*index);
                                        cx.notify();
                                    }))
                                    .children(ContentTab::ALL.into_iter().map(|tab| {
                                        let index = tab.index();
                                        Tab::new().label(format!(
                                            "{} ({})",
                                            tab.label(),
                                            counts.get(index).map_or(0, |(total, _)| *total)
                                        ))
                                    })),
                            )
                            .child(
                                Button::new("instance-browse-online")
                                    .outline()
                                    .icon(IconName::Globe)
                                    .label(tr("app.instance.download_content"))
                                    .disabled(pending)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_content_browser(
                                            this.content_tab.kind(),
                                            window,
                                            cx,
                                        )
                                    })),
                            )
                            .child(div().flex_1())
                            .child(
                                Input::new(&self.content_query)
                                    .prefix(Icon::new(IconName::Search).small())
                                    .cleanable(true)
                                    .w(rems(13.75)),
                            )
                            .child(
                                Button::new("instance-content-import")
                                    .primary()
                                    .w(rems(8.))
                                    .icon(IconName::Plus)
                                    .label(
                                        rust_i18n::t!(
                                            "app.instance.import_kind",
                                            kind = self.content_tab.label()
                                        )
                                        .to_string(),
                                    )
                                    .disabled(pending)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.pick_content_files(kind, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("instance-content-folder")
                                    .ghost()
                                    .icon(IconName::FolderOpen)
                                    .tooltip(tr("app.instance.open_content_folder"))
                                    .accessibility_label(tr("app.instance.open_content_folder"))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_content_folder(kind, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("instance-content-refresh")
                                    .ghost()
                                    .icon(IconName::RefreshCw)
                                    .tooltip(tr("app.instance.rescan_content"))
                                    .accessibility_label(tr("app.instance.rescan_content"))
                                    .disabled(pending)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.load_instance_content(window, cx)
                                    })),
                            ),
                    )
                    .child(if self.content_loading && library.is_none() {
                        card(cx)
                            .child(
                                h_flex()
                                    .w_full()
                                    .justify_center()
                                    .gap_2()
                                    .py_8()
                                    .child(
                                        Spinner::new()
                                            .with_size(Size::Small)
                                            .color(cx.theme().primary),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(tr("app.instance.scanning")),
                                    ),
                            )
                            .into_any_element()
                    } else if items.is_empty() {
                        card(cx)
                            .child(empty_state_message(
                                self.content_tab.icon(),
                                rust_i18n::t!(
                                    "app.instance.empty_kind",
                                    kind = self.content_tab.label()
                                )
                                .to_string(),
                                empty_description(kind),
                                cx,
                            ))
                            .into_any_element()
                    } else if kind == ContentKind::Mod {
                        card(cx)
                            .child(section_title(
                                self.content_tab.label(),
                                rust_i18n::t!(
                                    "app.instance.content_counts",
                                    total = content_count.0,
                                    enabled = content_count.1
                                )
                                .to_string(),
                                cx,
                            ))
                            .children(items.iter().map(|item| {
                                self.render_content_row(item, pending, cx)
                                    .into_any_element()
                            }))
                            .into_any_element()
                    } else if kind == ContentKind::ResourcePack {
                        self.render_resource_pack_gallery(
                            &items,
                            inspection,
                            content_count,
                            pending,
                            cx,
                        )
                    } else {
                        self.render_shader_pack_gallery(
                            &items,
                            inspection,
                            content_count,
                            pending,
                            cx,
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_instance_header(
        &self,
        version: &InstalledVersion,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = version.id.as_str().to_owned();
        let launch_id = id.clone();
        let folder_id = id.clone();
        let rename_id = id.clone();
        let loader_id = id.clone();
        let loader = version.loader.clone();
        h_flex()
            .w_full()
            .items_start()
            .gap_4()
            .child(
                Button::new("instance-back")
                    .ghost()
                    .icon(IconName::ArrowLeft)
                    .tooltip(tr("app.content_browser.back_instances"))
                    .accessibility_label(tr("app.content_browser.back_instances"))
                    .on_click(
                        cx.listener(|this, _, window, cx| {
                            this.navigate(Page::Versions, window, cx)
                        }),
                    ),
            )
            .child(version_tile(IconName::Package, cx.theme().primary, cx))
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
                                    .text_2xl()
                                    .font_semibold()
                                    .truncate()
                                    .child(version.display_name().to_owned()),
                            )
                            .when(!version.is_isolated(), |this| {
                                this.child(
                                    Tag::new().outline().child(tr("app.common.shared")).small(),
                                )
                            })
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
                            .child(type_tag(version.version_type.as_deref())),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "Minecraft {} · {} · {}",
                                version.minecraft_version,
                                version_type_label(version.version_type.as_deref()),
                                short_date(version.release_time.as_deref())
                            )),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .when(version.loader.is_none(), |this| {
                        let enabled = version.manifest_present
                            && version.manifest_error.is_none()
                            && self.can_start_operation();
                        let tooltip = if !version.manifest_present {
                            tr("app.instance.loader_legacy")
                        } else if version.manifest_error.is_some() {
                            tr("app.instance.loader_manifest_invalid")
                        } else if !self.can_start_operation() {
                            tr("app.instance.busy")
                        } else {
                            tr("app.instance.loader_help")
                        };
                        let install_id = loader_id.clone();
                        this.child(
                            Button::new("instance-install-loader")
                                .outline()
                                .icon(IconName::PackagePlus)
                                .label(tr("app.create_instance.add_loader_title"))
                                .disabled(!enabled)
                                .tooltip(tooltip)
                                .accessibility_label(tr("app.create_instance.add_loader_title"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_create_loader(install_id.clone(), window, cx)
                                })),
                        )
                    })
                    .child(
                        Button::new("instance-launch")
                            .outline()
                            .icon(IconName::Play)
                            .label(tr("app.home.launch"))
                            .disabled(
                                !self.log_state.can_start() || self.content_operations_pending(),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.start_launch(launch_id.clone(), window, cx)
                            })),
                    )
                    .child(
                        Button::new("instance-open-folder")
                            .ghost()
                            .icon(IconName::FolderOpen)
                            .tooltip(tr("app.instance.open_instance_folder"))
                            .accessibility_label(tr("app.instance.open_instance_folder"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_instance_folder(folder_id.as_str(), window, cx)
                            })),
                    )
                    .child(
                        Button::new("instance-rename")
                            .ghost()
                            .icon(IconName::Pencil)
                            .tooltip(tr("app.instance.rename"))
                            .accessibility_label(tr("app.instance.rename"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_rename_instance_dialog(rename_id.clone(), window, cx)
                            })),
                    )
                    .child(
                        Button::new("instance-refresh-all")
                            .ghost()
                            .icon(IconName::RefreshCw)
                            .tooltip(tr("app.instance.refresh"))
                            .accessibility_label(tr("app.instance.refresh"))
                            .disabled(self.content_operations_pending())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.reload_versions(window, cx);
                                this.load_instance_content(window, cx);
                            })),
                    ),
            )
    }

    fn render_instance_summary(
        &self,
        version: &InstalledVersion,
        counts: [(usize, usize); 3],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let instance = version.instance_layout(self.config.minecraft_root.clone());
        let java_target = version.id.as_str().to_owned();
        let auto_target = java_target.clone();
        let java_custom = version.java.is_some();
        let launch_target = java_target.clone();
        let java_label = version
            .java
            .as_ref()
            .map_or_else(|| tr("app.instance.java_auto"), display_path);
        let memory_label = version
            .max_memory_mb
            .map_or_else(|| tr("app.instance.auto"), |memory| format!("{memory} MiB"));
        let resolution_label = version.window_width.zip(version.window_height).map_or_else(
            || tr("app.instance.auto"),
            |(width, height)| format!("{width} × {height}"),
        );
        card(cx)
            .child(
                h_flex()
                    .w_full()
                    .items_start()
                    .gap_6()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(tr("app.instance.game_dir")),
                            )
                            .child(mono(display_path(instance.game_dir()), cx)),
                    )
                    .children(ContentTab::ALL.into_iter().map(|tab| {
                        let (total, enabled) = counts[tab.index()];
                        content_stat(tab.label(), total, enabled, cx)
                    })),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Java"),
                            )
                            .child(mono(java_label, cx)),
                    )
                    .child(
                        Button::new("instance-java-select")
                            .outline()
                            .icon(IconName::Package)
                            .label(tr("app.instance.select_java"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_java_dialog(java_target.clone(), window, cx)
                            })),
                    )
                    .when(java_custom, |this| {
                        this.child(
                            Button::new("instance-java-auto")
                                .ghost()
                                .label(tr("app.instance.java_reset"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.apply_instance_java(auto_target.clone(), None, window, cx);
                                })),
                        )
                    }),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(tr("app.instance.launch_settings")),
                            )
                            .child(mono(
                                rust_i18n::t!(
                                    "app.instance.launch_summary",
                                    memory = memory_label,
                                    resolution = resolution_label
                                )
                                .to_string(),
                                cx,
                            )),
                    )
                    .child(
                        Button::new("instance-launch-settings")
                            .outline()
                            .icon(IconName::Wrench)
                            .label(tr("app.instance.adjust"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_launch_settings_dialog(launch_target.clone(), window, cx)
                            })),
                    ),
            )
    }

    fn render_resource_pack_gallery(
        &self,
        items: &[LocalContent],
        inspection: Option<&ContentInspection>,
        counts: (usize, usize),
        pending: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let order = inspection
            .map(|inspection| inspection.resource_pack_order.as_slice())
            .unwrap_or(&[]);
        let mut items = items.to_vec();
        items.sort_by(|left, right| {
            resource_pack_sort_key(left, order).cmp(&resource_pack_sort_key(right, order))
        });
        let warning = inspection.and_then(|inspection| inspection.warnings.first().cloned());
        v_flex()
            .w_full()
            .gap_3()
            .child(section_title(
                super::super::content_kind_label(ContentKind::ResourcePack),
                rust_i18n::t!(
                    "app.instance.content_counts",
                    total = counts.0,
                    enabled = counts.1
                )
                .to_string(),
                cx,
            ))
            .when_some(warning, |this, warning| {
                this.child(warning_banner(
                    tr("app.instance.content_metadata"),
                    warning,
                    cx,
                ))
            })
            .child(
                div()
                    .w_full()
                    .grid()
                    .grid_cols(2)
                    .gap_4()
                    .children(items.iter().map(|item| {
                        self.render_resource_pack_card(item, inspection, order, pending, cx)
                            .into_any_element()
                    })),
            )
            .into_any_element()
    }

    fn render_resource_pack_card(
        &self,
        item: &LocalContent,
        inspection: Option<&ContentInspection>,
        order: &[String],
        pending: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let detail =
            inspection.and_then(|inspection| inspection.resource_packs.get(&item.relative_path));
        let title = pack_title(item);
        let description = detail
            .map(|detail| detail.metadata.description.clone())
            .filter(|description| !description.is_empty())
            .unwrap_or_else(|| tr("app.instance.no_description"));
        let format = resource_pack_format_label(detail.map(|detail| &detail.metadata));
        let incompatible = detail
            .is_some_and(|detail| detail.compatibility == ResourcePackCompatibility::Incompatible);
        let icon = self.content_icons.get(&item.relative_path);
        let position = order.iter().position(|name| name == item.file_name());
        let can_raise = item.enabled && position.is_some_and(|index| index + 1 < order.len());
        let can_lower = item.enabled && position.is_some_and(|index| index > 0);
        let toggle_relative = item.relative_path.clone();
        let raise_relative = item.relative_path.clone();
        let lower_relative = item.relative_path.clone();
        let delete_relative = item.relative_path.clone();
        let delete_name = title.clone();
        let enabled = item.enabled;
        let path = item.relative_path.as_str().to_owned();

        v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(if enabled {
                cx.theme().primary.opacity(0.30)
            } else {
                cx.theme().border
            })
            .bg(cx.theme().group_box)
            .p_3()
            .child(
                div()
                    .w_full()
                    .h_40()
                    .rounded(cx.theme().radius)
                    .overflow_hidden()
                    .bg(if enabled {
                        cx.theme().primary.opacity(0.10)
                    } else {
                        cx.theme().muted.opacity(0.35)
                    })
                    .when_some(icon.cloned(), |this, icon| {
                        this.child(img(icon).size_full().object_fit(ObjectFit::Cover))
                    })
                    .when(icon.is_none(), |this| {
                        this.flex().items_center().justify_center().child(
                            Icon::new(IconName::Image)
                                .with_size(Size::Large)
                                .text_color(cx.theme().muted_foreground),
                        )
                    }),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .font_semibold()
                                    .child(title),
                            )
                            .child(if incompatible {
                                Tag::danger().child(format).small().into_any_element()
                            } else {
                                Tag::new()
                                    .outline()
                                    .child(format)
                                    .small()
                                    .into_any_element()
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .line_clamp(2)
                            .child(description),
                    )
                    .when(incompatible, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().danger)
                                .child(tr("app.instance.incompatible")),
                        )
                    }),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_1()
                    .when(!enabled, |this| {
                        this.child(
                            Tag::new()
                                .outline()
                                .child(tr("app.instance.not_enabled"))
                                .small(),
                        )
                    })
                    .child(div().flex_1())
                    .when(enabled, |this| {
                        this.child(
                            Button::new(format!("resource-pack-raise-{path}"))
                                .ghost()
                                .xsmall()
                                .icon(IconName::ChevronUp)
                                .tooltip(tr("app.instance.raise_priority"))
                                .accessibility_label(tr("app.instance.raise_priority"))
                                .disabled(pending || !can_raise)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.change_resource_pack_priority(
                                        raise_relative.clone(),
                                        ResourcePackOrder::HigherPriority,
                                        window,
                                        cx,
                                    )
                                })),
                        )
                        .child(
                            Button::new(format!("resource-pack-lower-{path}"))
                                .ghost()
                                .xsmall()
                                .icon(IconName::ChevronDown)
                                .tooltip(tr("app.instance.lower_priority"))
                                .accessibility_label(tr("app.instance.lower_priority"))
                                .disabled(pending || !can_lower)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.change_resource_pack_priority(
                                        lower_relative.clone(),
                                        ResourcePackOrder::LowerPriority,
                                        window,
                                        cx,
                                    )
                                })),
                        )
                    })
                    .child(
                        Button::new(format!("resource-pack-toggle-{path}"))
                            .ghost()
                            .xsmall()
                            .icon(if enabled {
                                IconName::PowerOff
                            } else {
                                IconName::Power
                            })
                            .tooltip(if enabled {
                                tr("app.instance.disable")
                            } else {
                                tr("app.instance.enable")
                            })
                            .accessibility_label(if enabled {
                                tr("app.instance.disable")
                            } else {
                                tr("app.instance.enable")
                            })
                            .disabled(pending)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.toggle_content_item(
                                    ContentKind::ResourcePack,
                                    toggle_relative.clone(),
                                    !enabled,
                                    window,
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new(format!("resource-pack-delete-{path}"))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Trash)
                            .tooltip(tr("app.common.delete"))
                            .accessibility_label(tr("app.common.delete"))
                            .disabled(pending)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm_delete_content(
                                    ContentKind::ResourcePack,
                                    delete_relative.clone(),
                                    delete_name.clone(),
                                    window,
                                    cx,
                                )
                            })),
                    ),
            )
    }

    fn render_shader_pack_gallery(
        &self,
        items: &[LocalContent],
        inspection: Option<&ContentInspection>,
        counts: (usize, usize),
        pending: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let loader = inspection.and_then(|inspection| inspection.shader_loader);
        let selection = inspection.and_then(|inspection| inspection.shader_selection.as_ref());
        let active = selection.and_then(|selection| selection.active.as_deref());
        let active_enabled = selection.is_some_and(|selection| selection.enabled);
        let warning = inspection.and_then(|inspection| inspection.warnings.first().cloned());
        let mut items = items.to_vec();
        items.sort_by(|left, right| {
            let left_active = active_enabled && active == Some(left.file_name());
            let right_active = active_enabled && active == Some(right.file_name());
            right_active.cmp(&left_active).then_with(|| {
                left.display_name()
                    .to_lowercase()
                    .cmp(&right.display_name().to_lowercase())
            })
        });

        v_flex()
            .w_full()
            .gap_3()
            .child(section_title(
                super::super::content_kind_label(ContentKind::ShaderPack),
                rust_i18n::t!("app.instance.content_total", total = counts.0).to_string(),
                cx,
            ))
            .when_some(warning, |this, warning| {
                this.child(warning_banner(
                    tr("app.instance.content_metadata"),
                    warning,
                    cx,
                ))
            })
            .when(loader.is_none(), |this| {
                this.child(self.render_shader_loader_missing(cx))
            })
            .when_some(loader, |this, loader| {
                this.child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_2()
                        .rounded(cx.theme().radius)
                        .bg(cx.theme().primary.opacity(0.06))
                        .px_3()
                        .py_2()
                        .child(
                            Icon::new(IconName::Sparkles)
                                .small()
                                .text_color(cx.theme().primary),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(
                                    rust_i18n::t!(
                                        "app.instance.shader_loader_enabled",
                                        loader = loader.label()
                                    )
                                    .to_string(),
                                ),
                        ),
                )
            })
            .child(
                div()
                    .w_full()
                    .grid()
                    .grid_cols(2)
                    .gap_4()
                    .children(items.iter().map(|item| {
                        let is_active = active_enabled && active == Some(item.file_name());
                        self.render_shader_pack_card(
                            item,
                            inspection.and_then(|inspection| {
                                inspection.shader_packs.get(&item.relative_path)
                            }),
                            loader,
                            is_active,
                            pending,
                            cx,
                        )
                        .into_any_element()
                    })),
            )
            .into_any_element()
    }

    fn render_shader_pack_card(
        &self,
        item: &LocalContent,
        detail: Option<&ShaderPackMetadata>,
        loader: Option<ShaderLoader>,
        is_active: bool,
        pending: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let title = pack_title(item);
        let description = detail
            .and_then(|detail| detail.description.clone())
            .filter(|description| !description.is_empty())
            .unwrap_or_else(|| tr("app.instance.no_description"));
        let path = item.relative_path.as_str().to_owned();
        let select_path = item.file_name().to_owned();
        let delete_relative = item.relative_path.clone();
        let delete_name = title.clone();
        let loader_available = loader.is_some();

        v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(if is_active {
                cx.theme().primary.opacity(0.45)
            } else {
                cx.theme().border
            })
            .bg(cx.theme().group_box)
            .p_3()
            .child(
                div()
                    .w_full()
                    .h_40()
                    .rounded(cx.theme().radius)
                    .overflow_hidden()
                    .bg(if is_active {
                        cx.theme().primary.opacity(0.12)
                    } else {
                        cx.theme().muted.opacity(0.35)
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        Icon::new(IconName::Sparkles)
                            .with_size(Size::Large)
                            .text_color(if is_active {
                                cx.theme().primary
                            } else {
                                cx.theme().muted_foreground
                            }),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .font_semibold()
                                    .child(title),
                            )
                            .when(is_active, |this| {
                                this.child(Tag::primary().child(tr("app.instance.in_use")).small())
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .line_clamp(2)
                            .child(description),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if item.is_directory {
                                tr("app.instance.folder")
                            } else {
                                format_size(item.size.unwrap_or(0))
                            }),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_1()
                    .child(div().flex_1())
                    .child(if is_active {
                        Button::new(format!("shader-pack-disable-{path}"))
                            .outline()
                            .xsmall()
                            .icon(IconName::PowerOff)
                            .label(tr("app.instance.disable_shaders"))
                            .disabled(pending || !loader_available)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(loader) = loader {
                                    this.select_shader_pack(loader, None, window, cx);
                                }
                            }))
                    } else {
                        Button::new(format!("shader-pack-select-{path}"))
                            .outline()
                            .xsmall()
                            .icon(IconName::Sparkles)
                            .label(tr("app.instance.set_current"))
                            .disabled(pending || !loader_available)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(loader) = loader {
                                    this.select_shader_pack(
                                        loader,
                                        Some(select_path.clone()),
                                        window,
                                        cx,
                                    );
                                }
                            }))
                    })
                    .child(
                        Button::new(format!("shader-pack-delete-{path}"))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Trash)
                            .tooltip(tr("app.common.delete"))
                            .accessibility_label(tr("app.common.delete"))
                            .disabled(pending)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm_delete_content(
                                    ContentKind::ShaderPack,
                                    delete_relative.clone(),
                                    delete_name.clone(),
                                    window,
                                    cx,
                                )
                            })),
                    ),
            )
    }

    fn render_shader_loader_missing(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_2()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().warning.opacity(0.45))
            .bg(cx.theme().warning.opacity(0.08))
            .p_3()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Icon::new(IconName::TriangleAlert)
                            .small()
                            .text_color(cx.theme().warning),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_medium()
                            .child(tr("app.instance.shader_loader_missing")),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("app.instance.shader_loader_missing_help")),
            )
            .child(
                Button::new("shader-browse-loader-mods")
                    .outline()
                    .small()
                    .icon(IconName::Globe)
                    .label(tr("app.instance.find_loader"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_shader_loader_search(window, cx)
                    })),
            )
    }

    fn render_content_row(
        &self,
        item: &LocalContent,
        pending: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let kind = item.kind;
        let relative_path = item.relative_path.clone();
        let enabled = item.enabled;
        let can_toggle = item.can_toggle();
        let display_name = item.display_name().to_owned();
        let path = item.relative_path.as_str().to_owned();
        let meta = if item.is_directory {
            tr("app.instance.folder")
        } else {
            format_size(item.size.unwrap_or(0))
        };
        let toggle_relative = relative_path.clone();
        let delete_relative = relative_path.clone();
        let delete_name = display_name.clone();
        h_flex()
            .w_full()
            .gap_3()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background.opacity(0.35))
            .p_3()
            .child(version_tile(content_icon(kind), cx.theme().primary, cx))
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
                                    .text_sm()
                                    .font_semibold()
                                    .truncate()
                                    .child(display_name),
                            )
                            .when(item.is_directory, |this| {
                                this.child(
                                    Tag::new()
                                        .outline()
                                        .child(tr("app.instance.folder"))
                                        .small(),
                                )
                            })
                            .when(can_toggle && !enabled, |this| {
                                this.child(
                                    Tag::secondary().child(tr("app.instance.disabled")).small(),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{meta} · {path}")),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .when(can_toggle, |this| {
                        this.child(
                            Button::new(format!("content-toggle-{path}"))
                                .ghost()
                                .small()
                                .icon(if enabled {
                                    IconName::PowerOff
                                } else {
                                    IconName::Power
                                })
                                .tooltip(if enabled {
                                    tr("app.instance.disable")
                                } else {
                                    tr("app.instance.enable")
                                })
                                .accessibility_label(if enabled {
                                    tr("app.instance.disable")
                                } else {
                                    tr("app.instance.enable")
                                })
                                .disabled(pending)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.toggle_content_item(
                                        kind,
                                        toggle_relative.clone(),
                                        !enabled,
                                        window,
                                        cx,
                                    )
                                })),
                        )
                    })
                    .child(
                        Button::new(format!("content-delete-{path}"))
                            .ghost()
                            .small()
                            .icon(IconName::Trash)
                            .tooltip(tr("app.common.delete"))
                            .accessibility_label(tr("app.common.delete"))
                            .disabled(pending)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm_delete_content(
                                    kind,
                                    delete_relative.clone(),
                                    delete_name.clone(),
                                    window,
                                    cx,
                                )
                            })),
                    ),
            )
    }
}

fn pack_title(item: &LocalContent) -> String {
    let name = item.display_name();
    if name.to_ascii_lowercase().ends_with(".zip") {
        name[..name.len() - 4].to_owned()
    } else {
        name.to_owned()
    }
}

fn resource_pack_sort_key(item: &LocalContent, order: &[String]) -> (u8, usize, String) {
    let position = order.iter().position(|name| name == item.file_name());
    let group = if item.enabled && position.is_some() {
        0
    } else if item.enabled {
        1
    } else {
        2
    };
    let rank = position
        .map(|position| order.len().saturating_sub(position))
        .unwrap_or(0);
    (group, rank, item.display_name().to_lowercase())
}

fn resource_pack_format_label(metadata: Option<&ResourcePackMetadata>) -> String {
    let Some(metadata) = metadata else {
        return tr("app.instance.format_unknown");
    };
    if let Some(format) = metadata.pack_format {
        return rust_i18n::t!("app.instance.format", format = format).to_string();
    }
    match metadata.supported_formats.as_slice() {
        [range] if range.min_inclusive == range.max_inclusive => {
            rust_i18n::t!("app.instance.format", format = range.min_inclusive).to_string()
        }
        [range, ..] => rust_i18n::t!(
            "app.instance.format_range",
            min = range.min_inclusive,
            max = range.max_inclusive
        )
        .to_string(),
        [] => tr("app.instance.format_unknown"),
    }
}

fn content_icon(kind: ContentKind) -> IconName {
    match kind {
        ContentKind::Mod => IconName::Package,
        ContentKind::ResourcePack => IconName::Image,
        ContentKind::ShaderPack => IconName::Sparkles,
    }
}

fn content_stat(
    label: impl Into<SharedString>,
    total: usize,
    enabled: usize,
    cx: &App,
) -> impl IntoElement {
    let label = label.into();
    v_flex()
        .w(rems(7.))
        .flex_shrink_0()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label.clone()),
        )
        .child(div().text_lg().font_semibold().child(if enabled == total {
            total.to_string()
        } else {
            format!("{enabled}/{total}")
        }))
}

fn empty_description(kind: ContentKind) -> String {
    tr(match kind {
        ContentKind::Mod => "app.instance.empty_mods",
        ContentKind::ResourcePack => "app.instance.empty_resource_packs",
        ContentKind::ShaderPack => "app.instance.empty_shader_packs",
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rmcl_app::{
        ContentKind, LocalContent, PackFormat, PackFormatRange, RepositoryPath,
        ResourcePackMetadata,
    };

    use super::{pack_title, resource_pack_format_label, resource_pack_sort_key};

    fn local(name: &str, enabled: bool) -> LocalContent {
        LocalContent {
            kind: ContentKind::ResourcePack,
            relative_path: RepositoryPath::new(name).unwrap(),
            path: PathBuf::from(name),
            size: None,
            enabled,
            is_directory: false,
        }
    }

    #[test]
    fn resource_pack_sort_places_highest_priority_first() {
        let order = ["a.zip".to_owned(), "b.zip".to_owned(), "c.zip".to_owned()];
        let mut items = [
            local("a.zip", true),
            local("d.zip", false),
            local("c.zip", true),
            local("b.zip", true),
        ];
        items.sort_by(|left, right| {
            resource_pack_sort_key(left, &order).cmp(&resource_pack_sort_key(right, &order))
        });
        let names = items
            .iter()
            .map(LocalContent::display_name)
            .collect::<Vec<_>>();
        assert!(names == ["c.zip", "b.zip", "a.zip", "d.zip"]);
    }

    #[test]
    fn resource_pack_titles_and_format_labels_are_user_facing() {
        assert!(pack_title(&local("Cool Pack.ZIP", false)) == "Cool Pack");
        let metadata = ResourcePackMetadata {
            pack_format: Some(PackFormat::new(15, 0)),
            supported_formats: Vec::new(),
            description: String::new(),
            icon_png: None,
        };
        rust_i18n::set_locale("en");
        assert!(resource_pack_format_label(Some(&metadata)) == "Format 15");
        let range_metadata = ResourcePackMetadata {
            pack_format: None,
            supported_formats: vec![PackFormatRange::new(
                PackFormat::new(97, 1),
                PackFormat::new(97, 3),
            )],
            description: String::new(),
            icon_png: None,
        };
        assert!(resource_pack_format_label(Some(&range_metadata)) == "Format 97.1–97.3");
    }
}
