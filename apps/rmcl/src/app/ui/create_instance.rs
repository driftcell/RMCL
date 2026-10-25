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
        let loader_index = match self.create_loader {
            None => 0,
            Some(LoaderKind::Fabric) => 1,
            Some(LoaderKind::Quilt) => 2,
            Some(LoaderKind::Forge | LoaderKind::NeoForge) => 0,
        };
        let loader_ready = match self.create_loader {
            None => true,
            Some(LoaderKind::Fabric | LoaderKind::Quilt) => self.create_loader_version.is_some(),
            Some(LoaderKind::Forge | LoaderKind::NeoForge) => false,
        };
        let can_create = self.can_start_operation()
            && !self.create_base_input.read(cx).value().trim().is_empty()
            && !self.create_loader_loading
            && loader_ready;
        let loader_versions = self.create_loader_versions.clone();
        let selected_loader_version = self.create_loader_version.clone();
        let loading = self.create_loader_loading;
        let loader_error = self.create_loader_error.clone();

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
                            .child(div().text_2xl().font_semibold().child(Page::CreateInstance.title()))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(Page::CreateInstance.subtitle()),
                            ),
                    ),
            )
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
                                "基础版本会自动安装或复用；新实例默认启用版本隔离。",
                                cx,
                            ))
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(field_label("Minecraft 版本"))
                                    .child(
                                        Input::new(&self.create_base_input)
                                            .w_full()
                                            .max_w(px(360.)),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(
                                                "填写版本 ID，例如 1.21.1 或 24w14a。创建时从 Mojang manifest 校验。",
                                            ),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .gap_2()
                                    .child(field_label("加载器"))
                                    .child(
                                        TabBar::new("create-instance-loader")
                                            .segmented()
                                            .selected_index(loader_index)
                                            .on_click(cx.listener(
                                                |this, index: &usize, window, cx| {
                                                    let loader = match index {
                                                        1 => Some(LoaderKind::Fabric),
                                                        2 => Some(LoaderKind::Quilt),
                                                        _ => None,
                                                    };
                                                    this.select_create_loader(loader, window, cx);
                                                },
                                            ))
                                            .children(
                                                ["Vanilla", "Fabric", "Quilt"]
                                                    .into_iter()
                                                    .map(|label| Tab::new().label(label)),
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
                                                .as_deref()
                                                == Some(version.version.as_str());
                                            let value = version.version.clone();
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
                                                    format!("{} · 稳定版", value)
                                                } else {
                                                    value.clone()
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
                                            .label("创建实例")
                                            .disabled(!can_create)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.create_instance(window, cx)
                                            })),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(
                                                "Fabric/Quilt 会写入继承基础版本的实例 profile；Forge/NeoForge 暂未支持。",
                                            ),
                                    ),
                            ),
                    ),
            )
    }
}
