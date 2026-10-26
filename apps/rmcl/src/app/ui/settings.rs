use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Sizable, Size,
        button::{Button, ButtonVariants},
        h_flex,
        scroll::ScrollableElement,
        select::Select,
        spinner::Spinner,
        switch::Switch,
        tag::Tag,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};

use super::super::{LauncherApp, Page};
use super::common::{card, field_label, format_size, mono, page_header, section_title};

impl LauncherApp {
    pub fn render_settings(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content_width = (window.viewport_size().width - px(232.) - px(48.)).max(px(560.));
        let row_width = (content_width - px(40.)).max(px(400.));
        let auto_java_view = cx.entity().downgrade();
        let auto_update_view = cx.entity().downgrade();
        v_flex()
            .w_full()
            .items_stretch()
            .flex_1()
            .min_h_0()
            .p_6()
            .gap_6()
            .child(page_header(Page::Settings, div(), cx))
            .child(
                v_flex()
                    .id("settings-body")
                    .w(content_width)
                    .items_stretch()
                    .flex_1()
                    .min_h_0()
                    .gap_5()
                    .overflow_y_scrollbar()
                    .when_some(self.config_save_error.clone(), |this, error| {
                        this.child(
                            div()
                                .rounded(cx.theme().radius)
                                .border_1()
                                .border_color(cx.theme().danger.opacity(0.35))
                                .bg(cx.theme().danger.opacity(0.08))
                                .p_3()
                                .text_sm()
                                .text_color(cx.theme().danger)
                                .child(format!("配置保存失败：{error}")),
                        )
                    })
                    .child(
                        card(cx)
                            .child(section_title("游戏", "不会改写 HMCL 配置。", cx))
                            .child(
                                v_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(field_label("Minecraft 目录"))
                                    .child(
                                        h_flex()
                                            .w(row_width)
                                            .gap_2()
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .rounded(cx.theme().radius)
                                                    .border_1()
                                                    .border_color(cx.theme().border)
                                                    .bg(cx.theme().group_box)
                                                    .px_3()
                                                    .py_2()
                                                    .font_family(
                                                        cx.theme().mono_font_family.clone(),
                                                    )
                                                    .text_sm()
                                                    .child(
                                                        self.config
                                                            .minecraft_root
                                                            .display()
                                                            .to_string(),
                                                    ),
                                            )
                                            .child(
                                                Button::new("settings-browse-root")
                                                    .outline()
                                                    .icon(IconName::FolderOpen)
                                                    .label("选择目录")
                                                    .disabled(!self.log_state.can_start())
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            this.browse_minecraft_root(window, cx)
                                                        },
                                                    )),
                                            ),
                                    )
                                    .child(mono("更改后立即重新扫描版本，不移动现有文件。", cx)),
                            )
                            .child(
                                v_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(field_label("Java 目录"))
                                    .child(mono(self.config.java_store.display().to_string(), cx))
                                    .child(mono("重启 RMCL 后生效。", cx)),
                            )
                            .child(
                                h_flex()
                                    .w(row_width)
                                    .gap_4()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(field_label("自动获取 Java"))
                                            .child(mono(
                                                "缺少兼容 runtime 时自动下载；重启后生效。",
                                                cx,
                                            )),
                                    )
                                    .child(
                                        Switch::new("settings-auto-java")
                                            .flex_shrink_0()
                                            .checked(self.config.auto_install_java)
                                            .on_change(move |value, window, cx| {
                                                let _ = auto_java_view.update(cx, |this, cx| {
                                                    this.set_auto_java(*value, window, cx)
                                                });
                                            }),
                                    ),
                            ),
                    )
                    .child(
                        card(cx)
                            .child(section_title(
                                "启动行为",
                                "游戏运行时最小化启动器，退出或崩溃后自动恢复。",
                                cx,
                            ))
                            .child(
                                h_flex()
                                    .w(row_width)
                                    .gap_4()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(field_label("启动器可见性"))
                                            .child(mono("游戏退出后自动恢复。", cx)),
                                    )
                                    .child(
                                        div().flex_shrink_0().w(px(180.)).child(
                                            Select::new(&self.visibility_select)
                                                .accessibility_label("启动器可见性")
                                                .menu_width(px(200.))
                                                .menu_max_h(px(200.))
                                                .cleanable(false)
                                                .w_full(),
                                        ),
                                    ),
                            ),
                    )
                    .child(
                        card(cx)
                            .child(section_title("外观", "修改后立即生效。", cx))
                            .child(
                                h_flex()
                                    .w(row_width)
                                    .gap_4()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(field_label("主题"))
                                            .child(mono("跟随系统、浅色或深色。", cx)),
                                    )
                                    .child(
                                        div().flex_shrink_0().w(px(180.)).child(
                                            Select::new(&self.theme_select)
                                                .accessibility_label("主题")
                                                .menu_width(px(200.))
                                                .menu_max_h(px(200.))
                                                .cleanable(false)
                                                .w_full(),
                                        ),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .w(row_width)
                                    .gap_4()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(field_label("强调色"))
                                            .child(mono("主按钮和选中状态的颜色。", cx)),
                                    )
                                    .child(
                                        div().flex_shrink_0().w(px(180.)).child(
                                            Select::new(&self.accent_select)
                                                .accessibility_label("强调色")
                                                .menu_width(px(200.))
                                                .menu_max_h(px(200.))
                                                .cleanable(false)
                                                .w_full(),
                                        ),
                                    ),
                            ),
                    )
                    .child(
                        card(cx)
                            .child(section_title(
                                "更新",
                                "默认从 GitHub Release 检查；下载后校验 SHA-256 并自动重启。",
                                cx,
                            ))
                            .child(
                                h_flex()
                                    .w(row_width)
                                    .gap_4()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(field_label("当前版本"))
                                            .child(mono(
                                                format!("RMCL {}", env!("CARGO_PKG_VERSION")),
                                                cx,
                                            )),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(field_label("启动时检查更新"))
                                            .child(mono("自动检查发现新版本后提示。", cx)),
                                    )
                                    .child(
                                        Switch::new("settings-auto-update")
                                            .flex_shrink_0()
                                            .checked(self.config.auto_check_updates)
                                            .on_change(move |value, window, cx| {
                                                let _ = auto_update_view.clone().update(
                                                    cx,
                                                    |this, cx| {
                                                        this.set_auto_check_updates(
                                                            *value, window, cx,
                                                        )
                                                    },
                                                );
                                            }),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("settings-check-update")
                                            .outline()
                                            .icon(IconName::RefreshCw)
                                            .label("检查更新")
                                            .disabled(
                                                self.update.checking || self.update.downloading,
                                            )
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.check_for_updates(true, window, cx)
                                            })),
                                    )
                                    .when_some(self.update.available.clone(), |this, release| {
                                        this.child(
                                            Button::new("settings-install-update")
                                                .primary()
                                                .icon(IconName::Download)
                                                .label(format!("更新到 v{}", release.version))
                                                .disabled(
                                                    self.update.downloading
                                                        || !super::super::updates_installable(),
                                                )
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.open_update_dialog(window, cx)
                                                })),
                                        )
                                    })
                                    .when_some(self.update.available.as_ref(), |this, release| {
                                        this.child(
                                            Button::new("settings-update-notes")
                                                .ghost()
                                                .small()
                                                .icon(IconName::ExternalLink)
                                                .label("发布说明")
                                                .on_click({
                                                    let url = release.page_url.to_string();
                                                    move |_, _, cx| cx.open_url(&url)
                                                }),
                                        )
                                    })
                                    .when(self.update.checking, |this| {
                                        this.child(
                                            h_flex()
                                                .gap_2()
                                                .child(
                                                    Spinner::new()
                                                        .with_size(Size::XSmall)
                                                        .color(cx.theme().primary),
                                                )
                                                .child(mono("正在检查更新…", cx)),
                                        )
                                    }),
                            )
                            .when_some(self.update.progress, |this, progress| {
                                let label = match progress.total {
                                    Some(total) if total > 0 => format!(
                                        "下载中 {:.0}% · {} / {}",
                                        progress.downloaded as f64 / total as f64 * 100.0,
                                        format_size(progress.downloaded),
                                        format_size(total)
                                    ),
                                    _ => format!("下载中 · {}", format_size(progress.downloaded)),
                                };
                                this.child(mono(label, cx))
                            })
                            .when_some(self.update.status.clone(), |this, status| {
                                this.child(mono(status, cx))
                            })
                            .when_some(self.update.error.clone(), |this, error| {
                                this.child(
                                    div().text_sm().text_color(cx.theme().danger).child(error),
                                )
                            })
                            .when(!super::super::updates_installable(), |this| {
                                this.child(mono(
                                    "开发构建禁用手动安装；设置 RMCL_ALLOW_DEV_UPDATE=1 可测试。",
                                    cx,
                                ))
                            }),
                    )
                    .child(
                        card(cx)
                            .child(section_title("关于", "版本与许可。", cx))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(Tag::primary().child(env!("CARGO_PKG_VERSION")).small())
                                    .child(Tag::new().outline().child("MIT").small())
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("原生 Rust Minecraft 启动器"),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .w_full()
                                    .gap_1()
                                    .child(field_label("配置文件"))
                                    .child(mono(
                                        self.services.config_store.path().display().to_string(),
                                        cx,
                                    )),
                            )
                            .child(
                                v_flex()
                                    .w_full()
                                    .gap_1()
                                    .child(field_label("运行平台"))
                                    .child(mono(
                                        format!(
                                            "{} · Minecraft 由 Mojang Studios 提供",
                                            super::super::platform_label(&self.services.platform)
                                        ),
                                        cx,
                                    )),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("settings-gpui")
                                            .ghost()
                                            .small()
                                            .icon(IconName::ExternalLink)
                                            .label("GPUI Kit 文档")
                                            .on_click(|_, _, cx| {
                                                cx.open_url("https://gpui-kit.com");
                                            }),
                                    )
                                    .child(
                                        Button::new("settings-folder")
                                            .ghost()
                                            .small()
                                            .icon(IconName::FolderOpen)
                                            .label("打开游戏目录")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.open_game_folder(window, cx)
                                            })),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("RMCL 与 Mojang Studios 无关联。"),
                            ),
                    ),
            )
    }
}
