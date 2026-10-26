use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Sizable, Size, StyledExt,
        button::{Button, ButtonVariants},
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

use super::super::{LauncherApp, Page};
use super::common::{card, error_banner, field_label, section_title};

impl LauncherApp {
    pub fn render_create_instance(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let generated_id = self.generated_instance_id(cx);
        let entered_id = self.create_id_input.read(cx).value().trim().to_owned();
        let copy_mode = self.create_copy_source.is_some();
        let copy_source_name = self.create_copy_source.as_ref().and_then(|id| {
            self.installed_versions
                .iter()
                .find(|version| version.id.as_str() == id.as_str())
                .map(|version| version.display_name().to_owned())
        });
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
            && !self.create_base_input.read(cx).value().trim().is_empty()
            && !self.create_loader_loading
            && loader_ready
            && (!copy_mode || self.create_loader.is_some());
        let loader_versions = self.create_loader_versions.clone();
        let selected_loader_version = self.create_loader_version.clone();
        let loading = self.create_loader_loading;
        let loader_error = self.create_loader_error.clone();
        let page_title = if copy_mode {
            "添加 Mod 加载器"
        } else {
            Page::CreateInstance.title()
        };
        let page_subtitle = if copy_mode {
            "为现有实例创建新的加载器实例，并复制当前内容。"
        } else {
            Page::CreateInstance.subtitle()
        };
        let base_section_subtitle = if copy_mode {
            "基础版本来自现有实例；新实例会启用版本隔离。"
        } else {
            "基础版本会自动安装或复用；新实例默认启用版本隔离。"
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
                            .tooltip("返回实例列表")
                            .accessibility_label("返回实例列表")
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
                let name = copy_source_name.as_deref().unwrap_or("当前实例");
                this.child(
                    div()
                        .w_full()
                        .rounded(cx.theme().radius)
                        .border_1()
                        .border_color(cx.theme().primary.opacity(0.25))
                        .bg(cx.theme().primary.opacity(0.06))
                        .p_3()
                        .child(
                            div().text_sm().child(format!(
                                "正在为「{name}」添加 Mod 加载器。将创建新的隔离实例并复制当前实例的内容；原实例不会被修改。"
                            )),
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
                                "基础版本与加载器",
                                base_section_subtitle,
                                cx,
                            ))
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(field_label("Minecraft 版本"))
                                    .child(
                                        Input::new(&self.create_base_input)
                                            .w_full()
                                            .max_w(px(360.))
                                            .readonly(copy_mode),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(if copy_mode {
                                                "基础版本来自现有实例，不能修改。"
                                            } else {
                                                "填写版本 ID，例如 1.21.1 或 24w14a。创建时从 Mojang manifest 校验。"
                                            }),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .gap_2()
                                    .child(field_label("加载器"))
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
                                                .label("获取加载器版本")
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
                                                            .child("正在获取加载器版本…"),
                                                    ),
                                            )
                                        }),
                                )
                            })
                            .when_some(loader_error, |this, error: String| {
                                this.child(error_banner("加载器版本获取失败", &error, cx))
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
                                                .child("正在获取加载器版本…"),
                                        )
                                        .into_any_element()
                                } else if loader_versions.is_empty() {
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("填写 Minecraft 版本后点击“获取加载器版本”。")
                                        .into_any_element()
                                } else {
                                    v_flex()
                                        .w_full()
                                        .max_h(px(260.))
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
                                                    format!("{} · 稳定版", value.version)
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
                                "实例信息",
                                "名称和 ID 都可以留空；ID 会按基础版本和加载器自动生成。",
                                cx,
                            ))
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(field_label("显示名称"))
                                    .child(
                                        Input::new(&self.create_name_input)
                                            .w_full()
                                            .max_w(px(420.)),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(field_label("实例 ID"))
                                    .child(
                                        Input::new(&self.create_id_input)
                                            .w_full()
                                            .max_w(px(420.)),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(if entered_id.is_empty() {
                                                if generated_id.is_empty() {
                                                    "填写基础版本后自动生成。".to_owned()
                                                } else {
                                                    format!("自动生成的 ID：{generated_id}")
                                                }
                                            } else {
                                                format!("将使用自定义 ID：{entered_id}")
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
                                                "创建加载器实例"
                                            } else {
                                                "创建实例"
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
                                                "会创建新的隔离实例并复制当前实例内容；原实例不会被修改。"
                                            } else {
                                                "Fabric/Quilt 会写入继承基础版本的实例 profile；Forge/NeoForge 会运行官方 installer 处理器。"
                                            }),
                                    ),
                            ),
                    ),
            )
    }
}
