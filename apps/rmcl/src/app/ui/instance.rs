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
use rmcl_app::{ContentKind, InstalledVersion, LocalContent};

use super::super::{ContentTab, LauncherApp, Page, short_date, version_tile, version_type_label};
use super::common::{
    card, empty_state, error_banner, format_size, mono, section_title, type_tag, warning_banner,
};

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
                        .label("返回实例列表")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.navigate(Page::Versions, window, cx)
                        })),
                )
                .child(
                    card(cx).child(empty_state(
                        IconName::TriangleAlert,
                        "实例不存在",
                        "该实例可能已被删除或移动，请返回实例列表重新扫描。",
                        Button::new("instance-missing-back")
                            .primary()
                            .label("返回实例列表")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.navigate(Page::Versions, window, cx)
                            })),
                        cx,
                    )),
                )
                .into_any_element();
        };
        let library = self.content_library.as_ref();
        let items = self.filtered_content(cx);
        let pending = self.content_action_pending || self.content_loading;
        let kind = self.content_tab.kind();
        let counts = ContentTab::ALL.map(|tab| {
            let kind = tab.kind();
            let total = library.map_or(0, |library| library.count(kind));
            let enabled = library.map_or(0, |library| library.enabled_count(kind));
            (total, enabled)
        });
        let warning = library.and_then(|library| library.warnings.first());
        let warning_count = library.map_or(0, |library| library.warnings.len());
        let warning_title = format!("扫描警告（{warning_count}）");
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
                        this.child(error_banner("内容扫描失败", &error, cx))
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
                            .child(div().flex_1())
                            .child(
                                Input::new(&self.content_query)
                                    .prefix(Icon::new(IconName::Search).small())
                                    .cleanable(true)
                                    .w(px(220.)),
                            )
                            .child(
                                Button::new("instance-content-import")
                                    .primary()
                                    .icon(IconName::Plus)
                                    .label(format!("导入 {}", self.content_tab.label()))
                                    .disabled(pending)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.pick_content_files(kind, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("instance-content-folder")
                                    .ghost()
                                    .icon(IconName::FolderOpen)
                                    .tooltip("打开内容目录")
                                    .accessibility_label("打开内容目录")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_content_folder(kind, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("instance-content-refresh")
                                    .ghost()
                                    .icon(IconName::RefreshCw)
                                    .tooltip("重新扫描内容")
                                    .accessibility_label("重新扫描内容")
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
                                            .child("正在扫描实例内容…"),
                                    ),
                            )
                            .into_any_element()
                    } else if items.is_empty() {
                        card(cx)
                            .child(empty_state(
                                self.content_tab.icon(),
                                &format!("还没有 {}", self.content_tab.label()),
                                empty_description(kind),
                                Button::new("instance-empty-import")
                                    .primary()
                                    .icon(IconName::Plus)
                                    .label(format!("导入 {}", self.content_tab.label()))
                                    .disabled(pending)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.pick_content_files(kind, window, cx)
                                    })),
                                cx,
                            ))
                            .into_any_element()
                    } else {
                        card(cx)
                            .child(section_title(
                                self.content_tab.label(),
                                &format!(
                                    "共 {} 个，已启用 {} 个",
                                    content_count.0, content_count.1
                                ),
                                cx,
                            ))
                            .children(items.iter().map(|item| {
                                self.render_content_row(item, pending, cx)
                                    .into_any_element()
                            }))
                            .into_any_element()
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
        let loader = version.loader.clone();
        h_flex()
            .w_full()
            .items_start()
            .gap_4()
            .child(
                Button::new("instance-back")
                    .ghost()
                    .icon(IconName::ArrowLeft)
                    .tooltip("返回实例列表")
                    .accessibility_label("返回实例列表")
                    .on_click(
                        cx.listener(|this, _, window, cx| {
                            this.navigate(Page::Versions, window, cx)
                        }),
                    ),
            )
            .child(version_tile(IconName::Package, cx.theme().primary))
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
                            .child(if version.is_isolated() {
                                Tag::success().child("隔离").small()
                            } else {
                                Tag::new().outline().child("共享").small()
                            })
                            .when_some(loader, |this, loader| {
                                this.child(
                                    Tag::info()
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
                    .child(
                        Button::new("instance-browse-online")
                            .outline()
                            .icon(IconName::Globe)
                            .label("下载内容")
                            .disabled(self.content_action_pending || self.content_loading)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_content_browser(this.content_tab.kind(), window, cx)
                            })),
                    )
                    .child(
                        Button::new("instance-launch")
                            .outline()
                            .icon(IconName::Play)
                            .label("启动")
                            .disabled(
                                !self.log_state.can_start()
                                    || self.content_action_pending
                                    || self.content_loading,
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.start_launch(launch_id.clone(), window, cx)
                            })),
                    )
                    .child(
                        Button::new("instance-open-folder")
                            .ghost()
                            .icon(IconName::FolderOpen)
                            .tooltip("打开实例目录")
                            .accessibility_label("打开实例目录")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_instance_folder(folder_id.as_str(), window, cx)
                            })),
                    )
                    .child(
                        Button::new("instance-rename")
                            .ghost()
                            .icon(IconName::Pencil)
                            .tooltip("重命名实例")
                            .accessibility_label("重命名实例")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_rename_instance_dialog(rename_id.clone(), window, cx)
                            })),
                    )
                    .child(
                        Button::new("instance-refresh-all")
                            .ghost()
                            .icon(IconName::RefreshCw)
                            .tooltip("刷新实例信息")
                            .accessibility_label("刷新实例信息")
                            .disabled(self.content_action_pending || self.content_loading)
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
        let java_label = version.java.as_ref().map_or_else(
            || "自动选择兼容 runtime".to_owned(),
            |path| path.display().to_string(),
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
                                    .child("游戏目录"),
                            )
                            .child(mono(instance.game_dir().display().to_string(), cx)),
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
                            .label("选择 Java")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_java_dialog(java_target.clone(), window, cx)
                            })),
                    )
                    .when(java_custom, |this| {
                        this.child(
                            Button::new("instance-java-auto")
                                .ghost()
                                .label("改回自动")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.apply_instance_java(auto_target.clone(), None, window, cx);
                                })),
                        )
                    }),
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
            "文件夹".to_owned()
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
            .hover(|style| style.bg(cx.theme().list_hover))
            .child(version_tile(content_icon(kind), cx.theme().primary))
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
                                this.child(Tag::new().outline().child("文件夹").small())
                            })
                            .when_some(can_toggle.then_some(enabled), |this, enabled| {
                                this.child(if enabled {
                                    Tag::success().child("已启用").small()
                                } else {
                                    Tag::secondary().child("已禁用").small()
                                })
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
                                .tooltip(if enabled { "禁用" } else { "启用" })
                                .accessibility_label(if enabled { "禁用" } else { "启用" })
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
                            .tooltip("删除")
                            .accessibility_label("删除")
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

fn content_icon(kind: ContentKind) -> IconName {
    match kind {
        ContentKind::Mod => IconName::Package,
        ContentKind::ResourcePack => IconName::Image,
        ContentKind::ShaderPack => IconName::Sparkles,
    }
}

fn content_stat(label: &str, total: usize, enabled: usize, cx: &App) -> impl IntoElement {
    v_flex()
        .w(px(112.))
        .flex_shrink_0()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label.to_owned()),
        )
        .child(div().text_lg().font_semibold().child(if enabled == total {
            total.to_string()
        } else {
            format!("{enabled}/{total}")
        }))
}

fn empty_description(kind: ContentKind) -> &'static str {
    match kind {
        ContentKind::Mod => "支持 .jar、.zip 和 .litemod 文件；同名文件会自动改名，不会覆盖。",
        ContentKind::ResourcePack => {
            "支持 .zip 资源包；导入后可在列表中启用或禁用。文件夹资源包会显示已安装状态。"
        }
        ContentKind::ShaderPack => "支持 .zip 光影包或光影文件夹；光影没有统一的启用开关。",
    }
}
