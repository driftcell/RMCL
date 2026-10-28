use std::path::Path;

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
    ContentKind, ContentSource, InstalledVersion, RemoteProject, RemoteSort, RemoteVersion,
};

use super::super::{LauncherApp, Page, short_date, version_tile};
use super::common::{card, empty_state, error_banner, format_size, section_title, warning_banner};

impl LauncherApp {
    pub fn render_content_browser(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(version) = self.selected_instance_version().cloned() else {
            return v_flex()
                .flex_1()
                .min_h_0()
                .p_6()
                .gap_4()
                .child(
                    Button::new("content-browser-back-instance")
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
                        Button::new("content-browser-missing-back")
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
        let kind = self
            .content_browser
            .kind
            .unwrap_or_else(|| self.content_tab.kind());
        let project = self.content_browser.project.clone();
        let title = project
            .as_ref()
            .map(|project| project.title.clone())
            .unwrap_or_else(|| format!("在线 {}", kind.label()));
        let loader = version
            .loader
            .as_ref()
            .map(|loader| format!("{} {}", loader.kind.label(), loader.version));
        let subtitle = match loader {
            Some(loader) => format!(
                "Modrinth · Minecraft {} · {}",
                version.minecraft_version, loader
            ),
            None => format!("Modrinth · Minecraft {}", version.minecraft_version),
        };
        let refresh = project.clone();
        let refresh_versions = project.is_some();
        let project_icon = project
            .as_ref()
            .and_then(|project| self.content_browser.project_icons.get(&project.id))
            .and_then(|path| path.as_deref());
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
                        Button::new("content-browser-back")
                            .ghost()
                            .icon(IconName::ArrowLeft)
                            .tooltip("返回实例详情")
                            .accessibility_label("返回实例详情")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.back_to_instance_detail(window, cx)
                            })),
                    )
                    .child(project_tile(project_icon, kind, cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(div().text_2xl().font_semibold().truncate().child(title))
                                    .child(
                                        Tag::new()
                                            .outline()
                                            .child(ContentSource::Modrinth.label())
                                            .small(),
                                    ),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(subtitle),
                            ),
                    )
                    .when(project.is_some(), |this| {
                        this.child(
                            Button::new("content-browser-back-results")
                                .ghost()
                                .icon(IconName::ChevronLeft)
                                .label("返回搜索结果")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.back_to_content_search(cx)
                                })),
                        )
                    })
                    .child(
                        Button::new("content-browser-refresh")
                            .ghost()
                            .icon(IconName::RefreshCw)
                            .tooltip(if refresh_versions {
                                "刷新版本列表"
                            } else {
                                "重新搜索"
                            })
                            .accessibility_label(if refresh_versions {
                                "刷新版本列表"
                            } else {
                                "重新搜索"
                            })
                            .disabled(
                                self.content_browser.search_loading
                                    || self.content_browser.versions_loading,
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(project) = refresh.clone() {
                                    this.load_remote_versions(project, window, cx);
                                } else {
                                    this.load_remote_search(true, window, cx);
                                }
                            })),
                    ),
            )
            .child(
                v_flex()
                    .id("content-browser-body")
                    .flex_1()
                    .min_h_0()
                    .gap_4()
                    .overflow_y_scrollbar()
                    .when(
                        kind == ContentKind::Mod && version.loader.is_none(),
                        |this| {
                            this.child(warning_banner(
                                "当前实例没有 Mod 加载器",
                                "Mod 会下载到实例的 mods 目录，但在安装 Forge、Fabric、Quilt 或 NeoForge 前不会在游戏中生效。",
                                cx,
                            ))
                        },
                    )
                    .child(match project {
                        Some(project) => self
                            .render_remote_project(&project, &version, kind, cx)
                            .into_any_element(),
                        None => self
                            .render_remote_search(&version, kind, cx)
                            .into_any_element(),
                    }),
            )
            .into_any_element()
    }

    fn render_remote_search(
        &self,
        _version: &InstalledVersion,
        kind: ContentKind,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let loading = self.content_browser.search_loading;
        let projects = &self.content_browser.projects;
        let total = self.content_browser.total_hits;
        let error = self.content_browser.search_error.clone();
        let visible = projects.len() as u64;
        let count = if loading && projects.is_empty() {
            "正在搜索…".to_owned()
        } else {
            format!("共 {total} 个结果 · 已加载 {visible}")
        };
        v_flex()
            .gap_4()
            .child(
                card(cx).child(
                    v_flex()
                        .w_full()
                        .gap_3()
                        .child(
                            h_flex()
                                .w_full()
                                .gap_3()
                                .items_center()
                                .child(
                                    Input::new(&self.remote_query)
                                        .prefix(Icon::new(IconName::Search).small())
                                        .cleanable(true)
                                        .w(rems(18.75))
                                        .flex_shrink_0(),
                                )
                                .child(
                                    Button::new("content-browser-search")
                                        .primary()
                                        .icon(IconName::Search)
                                        .label("搜索")
                                        .disabled(loading)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.load_remote_search(true, window, cx)
                                        })),
                                )
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .whitespace_nowrap()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(count),
                                ),
                        )
                        .child(
                            TabBar::new("content-browser-sort")
                                .segmented()
                                .selected_index(remote_sort_index(self.remote_sort))
                                .on_click(cx.listener(|this, index: &usize, window, cx| {
                                    this.remote_sort =
                                        RemoteSort::ALL.get(*index).copied().unwrap_or_default();
                                    this.load_remote_search(true, window, cx);
                                }))
                                .children(
                                    RemoteSort::ALL
                                        .into_iter()
                                        .map(|sort| Tab::new().label(sort.label())),
                                ),
                        ),
                ),
            )
            .when_some(error, |this, error: String| {
                this.child(error_banner("Modrinth 搜索失败", &error, cx))
            })
            .child(if loading && projects.is_empty() {
                loading_card("正在搜索 Modrinth…", cx).into_any_element()
            } else if projects.is_empty() {
                card(cx)
                    .child(empty_state(
                        IconName::Globe,
                        "没有找到项目",
                        "尝试更换关键词，或确认当前实例的游戏版本和加载器过滤条件。",
                        Button::new("content-browser-empty-search")
                            .primary()
                            .icon(IconName::RefreshCw)
                            .label("重新搜索")
                            .disabled(loading)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.load_remote_search(true, window, cx)
                            })),
                        cx,
                    ))
                    .into_any_element()
            } else {
                let rows = projects
                    .iter()
                    .map(|project| {
                        self.render_remote_project_row(project, kind, cx)
                            .into_any_element()
                    })
                    .collect::<Vec<_>>();
                let result_subtitle = if loading {
                    format!("{} 个项目 · 正在搜索…", projects.len())
                } else {
                    format!("{} 个项目", projects.len())
                };
                card(cx)
                    .child(section_title("搜索结果", &result_subtitle, cx))
                    .children(rows)
                    .when(visible < total, |this| {
                        this.child(
                            Button::new("content-browser-load-more")
                                .outline()
                                .w_full()
                                .label(if loading {
                                    "正在加载…"
                                } else {
                                    "加载更多"
                                })
                                .disabled(loading)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.load_remote_search(false, window, cx)
                                })),
                        )
                    })
                    .into_any_element()
            })
            .into_any_element()
    }

    fn render_remote_project_row(
        &self,
        project: &RemoteProject,
        kind: ContentKind,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let project_for_click = project.clone();
        let icon_path = self
            .content_browser
            .project_icons
            .get(&project.id)
            .and_then(|path| path.as_deref());
        let categories = project
            .categories
            .iter()
            .filter(|category| !is_loader(category))
            .take(4)
            .cloned()
            .collect::<Vec<_>>();
        h_flex()
            .w_full()
            .gap_3()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background.opacity(0.35))
            .p_3()
            .child(project_tile(icon_path, kind, cx))
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
                                    .child(project.title.clone()),
                            )
                            .when(!project.author.is_empty(), |this| {
                                this.child(
                                    Tag::new()
                                        .outline()
                                        .child(format!("by {}", project.author))
                                        .small(),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .truncate()
                            .child(project.description.clone()),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .child(
                                Tag::new()
                                    .outline()
                                    .child(format!("{} 次下载", format_count(project.downloads)))
                                    .small(),
                            )
                            .children(
                                project.loaders.iter().map(|loader| {
                                    Tag::new().outline().child(loader.clone()).small()
                                }),
                            )
                            .children(
                                categories
                                    .into_iter()
                                    .map(|category| Tag::secondary().child(category).small()),
                            ),
                    ),
            )
            .child(
                Button::new(format!("content-browser-project-{}", project.id))
                    .outline()
                    .small()
                    .icon(IconName::List)
                    .label("查看版本")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_remote_project(project_for_click.clone(), window, cx)
                    })),
            )
    }

    fn render_remote_project(
        &self,
        project: &RemoteProject,
        _version: &InstalledVersion,
        kind: ContentKind,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let loading = self.content_browser.versions_loading;
        let versions = &self.content_browser.versions;
        let error = self.content_browser.versions_error.clone();
        let categories = project
            .categories
            .iter()
            .filter(|category| !is_loader(category))
            .take(6)
            .cloned()
            .collect::<Vec<_>>();
        v_flex()
            .gap_4()
            .child(
                card(cx).child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(div().text_lg().font_semibold().child(project.title.clone()))
                                .when(!project.author.is_empty(), |this| {
                                    this.child(
                                        Tag::new()
                                            .outline()
                                            .child(format!("by {}", project.author))
                                            .small(),
                                    )
                                })
                                .child(
                                    Tag::new()
                                        .outline()
                                        .child(format!(
                                            "{} 次下载",
                                            format_count(project.downloads)
                                        ))
                                        .small(),
                                ),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(project.description.clone()),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .flex_wrap()
                                .children(project.loaders.iter().map(|loader| {
                                    Tag::new().outline().child(loader.clone()).small()
                                }))
                                .children(
                                    categories
                                        .into_iter()
                                        .map(|category| Tag::secondary().child(category).small()),
                                )
                                .when_some(project.updated.as_deref(), |this, updated| {
                                    this.child(
                                        Tag::new()
                                            .outline()
                                            .child(format!("更新于 {}", short_date(Some(updated))))
                                            .small(),
                                    )
                                }),
                        ),
                ),
            )
            .when_some(error, |this, error: String| {
                this.child(error_banner("Modrinth 版本加载失败", &error, cx))
            })
            .child(if loading && versions.is_empty() {
                loading_card("正在加载版本列表…", cx).into_any_element()
            } else if versions.is_empty() {
                card(cx)
                    .child(empty_state(
                        IconName::Package,
                        "没有匹配版本",
                        "该项目的版本可能不支持当前实例的 Minecraft 版本或加载器。",
                        Button::new("content-browser-versions-back")
                            .primary()
                            .icon(IconName::ChevronLeft)
                            .label("返回搜索结果")
                            .on_click(
                                cx.listener(|this, _, _, cx| this.back_to_content_search(cx)),
                            ),
                        cx,
                    ))
                    .into_any_element()
            } else {
                let rows = versions
                    .iter()
                    .map(|version| {
                        self.render_remote_version_row(version, kind, cx)
                            .into_any_element()
                    })
                    .collect::<Vec<_>>();
                card(cx)
                    .child(section_title(
                        "可用版本",
                        &format!("{} 个版本", versions.len()),
                        cx,
                    ))
                    .children(rows)
                    .into_any_element()
            })
            .into_any_element()
    }

    fn render_remote_version_row(
        &self,
        version: &RemoteVersion,
        kind: ContentKind,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let file = version.installable_file(kind);
        let recommended = self.content_browser.recommended.as_deref() == Some(version.id.as_str());
        let can_install = self.can_start_operation()
            && !self.content_browser.preflighting
            && file.is_some_and(|file| file.sha1.is_some());
        let file_label = file
            .map(|file| {
                format!(
                    "{} · {}",
                    file.filename,
                    file.size
                        .map(format_size)
                        .unwrap_or_else(|| "大小未知".to_owned())
                )
            })
            .unwrap_or_else(|| "没有可用文件".to_owned());
        let dependencies = version.required_dependencies().count();
        let version_for_click = version.clone();
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
                            .child(div().text_sm().font_semibold().truncate().child(
                                if version.name.is_empty() {
                                    version.version_number.clone()
                                } else {
                                    version.name.clone()
                                },
                            ))
                            .child(version_type_tag(&version.version_type))
                            .when(recommended, |this| {
                                this.child(Tag::success().child("推荐").small())
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "{} · {} · {}",
                                version.version_number,
                                short_date(version.published.as_deref()),
                                if version.game_versions.is_empty() {
                                    "游戏版本未知".to_owned()
                                } else {
                                    version.game_versions.join(", ")
                                }
                            )),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if version.loaders.is_empty() {
                                file_label.clone()
                            } else {
                                format!("{} · {}", version.loaders.join(", "), file_label)
                            }),
                    )
                    .when(dependencies > 0, |this| {
                        this.child(
                            Tag::warning()
                                .child(format!("需要 {dependencies} 个前置"))
                                .small(),
                        )
                    }),
            )
            .child(
                Button::new(format!("content-browser-install-{}", version.id))
                    .primary()
                    .small()
                    .icon(IconName::Download)
                    .label("安装")
                    .disabled(!can_install)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.install_remote_version(version_for_click.clone(), window, cx)
                    })),
            )
    }
}

fn remote_sort_index(sort: RemoteSort) -> usize {
    RemoteSort::ALL
        .iter()
        .position(|candidate| *candidate == sort)
        .unwrap_or(0)
}

fn project_tile(path: Option<&Path>, kind: ContentKind, cx: &App) -> AnyElement {
    let Some(path) = path else {
        return version_tile(content_icon(kind), cx.theme().primary, cx).into_any_element();
    };
    div()
        .size_11()
        .flex_shrink_0()
        .rounded(cx.theme().radius_tokens().xl)
        .overflow_hidden()
        .bg(cx.theme().primary.opacity(0.12))
        .child(
            img(path.to_path_buf())
                .size_full()
                .object_fit(ObjectFit::Cover),
        )
        .into_any_element()
}

fn content_icon(kind: ContentKind) -> IconName {
    match kind {
        ContentKind::Mod => IconName::Package,
        ContentKind::ResourcePack => IconName::Image,
        ContentKind::ShaderPack => IconName::Sparkles,
    }
}

fn is_loader(value: &str) -> bool {
    matches!(value, "fabric" | "quilt" | "forge" | "neoforge")
}

fn version_type_tag(version_type: &str) -> Tag {
    let label = match version_type {
        "release" => "正式版",
        "beta" => "Beta",
        "alpha" => "Alpha",
        other => other,
    };
    Tag::new().outline().child(label.to_owned()).small()
}

fn format_count(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1_000.)
    } else {
        value.to_string()
    }
}

fn loading_card(message: &str, cx: &App) -> impl IntoElement {
    card(cx).child(
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
                    .child(message.to_owned()),
            ),
    )
}
