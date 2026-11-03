use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Sizable, Size, StyledExt,
        button::{Button, ButtonVariants},
        combobox::Combobox,
        h_flex,
        input::Input,
        scroll::ScrollableElement,
        spinner::Spinner,
        tab::{Tab, TabBar},
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use rmcl_app::LoaderKind;

use super::super::{LauncherApp, Page, short_date, version_type_label};
use super::common::{card, error_banner, field_label, section_title};
use super::layout;
use crate::i18n::tr;

impl LauncherApp {
    pub fn render_create_instance(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let generated_id = self.generated_instance_id(cx);
        let entered_id = self.create_id_input.read(cx).value().trim().to_owned();
        let base = self.create_base_input.read(cx).value().trim().to_owned();
        let copy_mode = self.create_copy_source.is_some();
        let copy_source_name = self.create_copy_source.as_ref().and_then(|id| {
            self.installed_versions
                .iter()
                .find(|version| version.id.as_str() == id.as_str())
                .map(|version| version.display_name().to_owned())
        });
        let catalog_ready = self.catalog.manifest.is_some();
        let catalog_loading = self.catalog.loading;
        let catalog_error = if catalog_ready {
            None
        } else {
            self.catalog.error.clone()
        };
        let base_hint = if copy_mode {
            tr("app.create_instance.base_fixed")
        } else if let Some(version) = self
            .catalog
            .manifest
            .as_ref()
            .and_then(|manifest| manifest.find(&base))
        {
            format!(
                "{} · {}",
                version_type_label(version.version_type.as_deref()),
                short_date(version.release_time.as_deref())
            )
        } else if catalog_loading && !catalog_ready {
            tr("app.create_instance.catalog_loading")
        } else if catalog_ready {
            tr("app.create_instance.catalog_hint")
        } else {
            tr("app.create_instance.catalog_failed_hint")
        };
        let loader_index: usize = match self.create_loader {
            None => 0,
            Some(LoaderKind::Fabric) => 1,
            Some(LoaderKind::Quilt) => 2,
            Some(LoaderKind::Forge) => 3,
            Some(LoaderKind::NeoForge) => 4,
        };
        let selected_loader_index = if copy_mode {
            loader_index.saturating_sub(1)
        } else {
            loader_index
        };
        let loader_tabs: &[&str] = if copy_mode {
            &["Fabric", "Quilt", "Forge", "NeoForge"]
        } else {
            &["Vanilla", "Fabric", "Quilt", "Forge", "NeoForge"]
        };
        let loader_ready = self.create_loader.is_none() || self.create_loader_version.is_some();
        let can_create = self.can_start_operation()
            && !base.is_empty()
            && !self.create_loader_loading
            && loader_ready
            && (!copy_mode || self.create_loader.is_some());
        let loader_versions = self.create_loader_versions.clone();
        let selected_loader_version = self.create_loader_version.clone();
        let loading = self.create_loader_loading;
        let loader_error = self.create_loader_error.clone();
        let page_title = if copy_mode {
            tr("app.create_instance.add_loader_title")
        } else {
            Page::CreateInstance.title()
        };
        let page_subtitle = if copy_mode {
            tr("app.create_instance.add_loader_subtitle")
        } else {
            Page::CreateInstance.subtitle()
        };
        let base_section_subtitle = if copy_mode {
            tr("app.create_instance.base_from_existing")
        } else {
            tr("app.create_instance.base_auto")
        };

        v_flex()
            .flex_1()
            .min_h_0()
            .p_6()
            .gap_5()
            .child(
                h_flex()
                    .w_full()
                    .items_start()
                    .gap_3()
                    .child(
                        Button::new("create-instance-back")
                            .ghost()
                            .icon(IconName::ArrowLeft)
                            .tooltip(tr("app.create_instance.back"))
                            .accessibility_label(tr("app.create_instance.back"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.navigate(Page::Versions, window, cx)
                            })),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_2xl().font_semibold().child(page_title))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(page_subtitle),
                            ),
                    ),
            )
            .when(copy_mode, |this| {
                let name = copy_source_name
                    .as_deref()
                    .map(str::to_owned)
                    .unwrap_or_else(|| tr("app.create_instance.current_instance"));
                this.child(
                    div()
                        .w_full()
                        .rounded(cx.theme().radius)
                        .border_1()
                        .border_color(cx.theme().primary.opacity(0.25))
                        .bg(cx.theme().primary.opacity(0.06))
                        .p_3()
                        .child(
                            div().text_sm().child(
                                rust_i18n::t!("app.create_instance.copy_banner", name = name)
                                    .to_string(),
                            ),
                        ),
                )
            })
            .child(
                v_flex()
                    .id("create-instance-body")
                    .flex_1()
                    .min_h_0()
                    .gap_4()
                    .overflow_y_scrollbar()
                    .child(
                        card(cx)
                            .child(section_title(
                                tr("app.create_instance.base_and_loader"),
                                base_section_subtitle,
                                cx,
                            ))
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(field_label(tr("app.create_instance.minecraft_version")))
                                    .when(copy_mode, |this| {
                                        this.child(
                                            h_flex()
                                                .w_full()
                                                .max_w(rems(22.5))
                                                .h_8()
                                                .px_3()
                                                .rounded(cx.theme().radius)
                                                .border_1()
                                                .border_color(cx.theme().border)
                                                .bg(cx.theme().muted.opacity(0.45))
                                                .child(div().text_sm().child(base.clone())),
                                        )
                                    })
                                    .when(!copy_mode, |this| {
                                        this.child(
                                            div()
                                                .w_full()
                                                .max_w(rems(22.5))
                                                .when(catalog_ready, |this| {
                                                    this.child(
                                                        Combobox::new(
                                                            &self.create_version_select,
                                                        )
                                                        .w_full()
                                                        .placeholder(
                                                            tr("app.create_instance.select_version"),
                                                        )
                                                        .search_placeholder(
                                                            tr("app.create_instance.search_version"),
                                                        )
                                                        .menu_max_h(layout::rem_px(cx, rems(20.))),
                                                    )
                                                })
                                                .when(catalog_loading && !catalog_ready, |this| {
                                                    this.child(
                                                        Combobox::new(
                                                            &self.create_version_select,
                                                        )
                                                        .w_full()
                                                        .placeholder(
                                                            tr("app.create_instance.loading_versions"),
                                                        )
                                                        .disabled(true),
                                                    )
                                                })
                                                .when(!catalog_loading && !catalog_ready, |this| {
                                                    this.child(
                                                        Input::new(&self.create_base_input).w_full(),
                                                    )
                                                }),
                                        )
                                        .when_some(catalog_error, |this, error| {
                                            this.child(error_banner(
                                                tr("app.create_instance.catalog_load_failed"),
                                                error,
                                                cx,
                                            ))
                                            .child(
                                                Button::new("create-instance-retry-catalog")
                                                    .outline()
                                                    .small()
                                                    .icon(IconName::RefreshCw)
                                                    .label(tr("app.create_instance.retry_catalog"))
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            this.load_catalog(true, window, cx)
                                                        },
                                                    )),
                                            )
                                        })
                                    })
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(base_hint),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .gap_2()
                                    .child(field_label(tr("app.create_instance.loader")))
                                    .child(
                                        TabBar::new("create-instance-loader")
                                            .segmented()
                                            .selected_index(selected_loader_index)
                                            .on_click(cx.listener(
                                                move |this, index: &usize, window, cx| {
                                                    let loader = if copy_mode {
                                                        match index {
                                                            0 => Some(LoaderKind::Fabric),
                                                            1 => Some(LoaderKind::Quilt),
                                                            2 => Some(LoaderKind::Forge),
                                                            3 => Some(LoaderKind::NeoForge),
                                                            _ => None,
                                                        }
                                                    } else {
                                                        match index {
                                                            1 => Some(LoaderKind::Fabric),
                                                            2 => Some(LoaderKind::Quilt),
                                                            3 => Some(LoaderKind::Forge),
                                                            4 => Some(LoaderKind::NeoForge),
                                                            _ => None,
                                                        }
                                                    };
                                                    this.select_create_loader(loader, window, cx);
                                                },
                                            ))
                                            .children(
                                                loader_tabs
                                                    .iter()
                                                    .map(|label| Tab::new().label(*label)),
                                            ),
                                    ),
                            )
                            .when(self.create_loader.is_some(), |this| {
                                this.child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            Button::new("create-instance-fetch-loaders")
                                                .outline()
                                                .icon(IconName::RefreshCw)
                                                .label(tr("app.create_instance.fetch_loader_versions"))
                                                .disabled(
                                                    loading
                                                        || self
                                                            .create_base_input
                                                            .read(cx)
                                                            .value()
                                                            .trim()
                                                            .is_empty(),
                                                )
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.fetch_loader_versions(window, cx)
                                                })),
                                        )
                                        .when(loading, |this| {
                                            this.child(
                                                h_flex()
                                                    .gap_2()
                                                    .child(
                                                        Spinner::new()
                                                            .with_size(Size::Small)
                                                            .color(cx.theme().primary),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(
                                                                cx.theme().muted_foreground,
                                                            )
                                                            .child(
                                                                tr("app.create_instance.loading_loader_versions"),
                                                            ),
                                                    ),
                                            )
                                        }),
                                )
                            })
                            .when_some(loader_error, |this, error: String| {
                                this.child(error_banner(
                                    tr("app.create_instance.loader_load_failed"),
                                    error,
                                    cx,
                                ))
                            })
                            .when(self.create_loader.is_some(), |this| {
                                this.child(if loading && loader_versions.is_empty() {
                                    h_flex()
                                        .w_full()
                                        .justify_center()
                                        .py_6()
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(
                                                    tr("app.create_instance.loading_loader_versions"),
                                                ),
                                        )
                                        .into_any_element()
                                } else if loader_versions.is_empty() {
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(tr("app.create_instance.loader_versions_hint"))
                                        .into_any_element()
                                } else {
                                    v_flex()
                                        .w_full()
                                        .max_h(layout::MENU_MAX_HEIGHT)
                                        .gap_1()
                                        .overflow_y_scrollbar()
                                        .children(loader_versions.into_iter().map(|version| {
                                            let selected = selected_loader_version
                                                .as_ref()
                                                .is_some_and(|selected| {
                                                    selected.version == version.version
                                                });
                                            let value = version.clone();
                                            let id = format!(
                                                "create-loader-version-{}",
                                                version.version
                                            );
                                            let button = if selected {
                                                Button::new(id).primary()
                                            } else {
                                                Button::new(id).outline()
                                            };
                                            button
                                                .w_full()
                                                .small()
                                                .label(if version.stable {
                                                    rust_i18n::t!(
                                                        "app.create_instance.stable_version",
                                                        version = value.version
                                                    )
                                                    .to_string()
                                                } else {
                                                    value.version.clone()
                                                })
                                                .on_click(cx.listener(
                                                    move |this, _, _, cx| {
                                                        this.create_loader_version =
                                                            Some(value.clone());
                                                        cx.notify();
                                                    },
                                                ))
                                                .into_any_element()
                                        }))
                                        .into_any_element()
                                })
                            }),
                    )
                    .child(
                        card(cx)
                            .child(section_title(
                                tr("app.create_instance.instance_info"),
                                tr("app.create_instance.instance_info_hint"),
                                cx,
                            ))
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(field_label(tr("app.create_instance.display_name")))
                                    .child(
                                        Input::new(&self.create_name_input)
                                            .w_full()
                                            .max_w(layout::DIALOG_SMALL),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(field_label(tr("app.create_instance.instance_id")))
                                    .child(
                                        Input::new(&self.create_id_input)
                                            .w_full()
                                            .max_w(layout::DIALOG_SMALL),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(if entered_id.is_empty() {
                                                if generated_id.is_empty() {
                                                    tr("app.create_instance.id_auto_hint")
                                                } else {
                                                    rust_i18n::t!(
                                                        "app.create_instance.generated_id",
                                                        id = generated_id
                                                    )
                                                    .to_string()
                                                }
                                            } else {
                                                rust_i18n::t!(
                                                    "app.create_instance.custom_id",
                                                    id = entered_id
                                                )
                                                .to_string()
                                            }),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("create-instance-submit")
                                            .primary()
                                            .large()
                                            .icon(IconName::PackagePlus)
                                            .label(if copy_mode {
                                                tr("app.create_instance.create_loader_instance")
                                            } else {
                                                tr("app.create_instance.create_instance")
                                            })
                                            .disabled(!can_create)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.create_instance(window, cx)
                                            })),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(if copy_mode {
                                                tr("app.create_instance.copy_notice")
                                            } else {
                                                tr("app.create_instance.loader_notice")
                                            }),
                                    ),
                            ),
                    ),
            )
    }
}
