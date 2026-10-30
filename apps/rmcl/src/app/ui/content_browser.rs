use std::path::Path;

use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon, Sizable, Size, StyledExt,
        button::{Button, ButtonVariants},
        carousel::{
            Carousel, CarouselContent, CarouselItem, CarouselNext, CarouselPagination,
            CarouselPaginationItem, CarouselPrevious,
        },
        h_flex,
        input::Input,
        link::Link,
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

use super::super::{ContentTab, LauncherApp, Page, short_date, version_tile};
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
        let kind_tab = ContentTab::from_kind(kind);
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
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.back_to_content_search(window, cx)
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
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_3()
                    .child(
                        TabBar::new("content-browser-kind")
                            .segmented()
                            .selected_index(kind_tab.index())
                            .on_click(cx.listener(|this, index: &usize, window, cx| {
                                this.switch_content_browser_kind(
                                    ContentTab::from_index(*index).kind(),
                                    window,
                                    cx,
                                )
                            }))
                            .children(
                                ContentTab::ALL
                                    .into_iter()
                                    .map(|tab| Tab::new().label(tab.label())),
                            ),
                    )
                    .child(div().flex_1()),
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
                    .when(
                        kind == ContentKind::ShaderPack
                            && self.content_browser.shader_loader.is_none(),
                        |this| {
                            this.child(warning_banner(
                                "当前实例没有光影加载器",
                                "下载后需要 Iris、Oculus 或 OptiFine 才能生效。",
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
            } else if is_visual_kind(kind) {
                self.render_visual_project_results(projects, visible, total, loading, cx)
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
                        this.child(load_more_button(loading, cx))
                    })
                    .into_any_element()
            })
            .into_any_element()
    }

    fn render_visual_project_results(
        &self,
        projects: &[RemoteProject],
        visible: u64,
        total: u64,
        loading: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let kind = self
            .content_browser
            .kind
            .unwrap_or(ContentKind::ResourcePack);
        let subtitle = if loading {
            format!("已加载 {} 个项目 · 正在搜索…", projects.len())
        } else {
            format!("已加载 {} 个项目", projects.len())
        };
        v_flex()
            .w_full()
            .gap_4()
            .child(section_title("搜索结果", &subtitle, cx))
            .child(
                div()
                    .w_full()
                    .grid()
                    .grid_cols(2)
                    .gap_4()
                    .children(projects.iter().map(|project| {
                        self.render_visual_project_card(project, kind, cx)
                            .into_any_element()
                    })),
            )
            .when(visible < total, |this| {
                this.child(load_more_button(loading, cx))
            })
    }

    fn render_visual_project_card(
        &self,
        project: &RemoteProject,
        kind: ContentKind,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let preview = project.preview_url().and_then(|url| {
            self.content_browser
                .project_previews
                .get(url)
                .and_then(|path| path.as_deref())
        });
        let project_for_click = project.clone();
        v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().group_box)
            .p_3()
            .child(preview_canvas(preview, kind, false, cx))
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .text_base()
                            .font_semibold()
                            .child(project.title.clone()),
                    )
                    .when(!project.author.is_empty(), |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("作者：{}", project.author)),
                        )
                    })
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .line_clamp(2)
                            .child(project.description.clone()),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_3()
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(Icon::new(IconName::Download).with_size(Size::XSmall))
                            .child(format!("{} 次下载", format_count(project.downloads))),
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
                    ),
            )
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

    fn render_visual_project_overview(
        &self,
        project: &RemoteProject,
        kind: ContentKind,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let recommended = self.content_browser.recommended.as_deref().and_then(|id| {
            self.content_browser
                .versions
                .iter()
                .find(|version| version.id == id)
        });
        let recommended_name = recommended.map(|version| {
            if version.name.is_empty() {
                version.version_number.clone()
            } else {
                version.name.clone()
            }
        });
        let recommended_file = recommended
            .and_then(|version| version.installable_file(kind))
            .map(|file| {
                format!(
                    "{} · {}",
                    file.filename,
                    file.size
                        .map(format_size)
                        .unwrap_or_else(|| "大小未知".to_owned())
                )
            });
        let install_version = recommended.cloned();
        let has_recommended = install_version.is_some();
        let can_install = recommended.is_some()
            && self.can_start_operation()
            && !self.content_browser.preflighting;
        let gallery = self.render_project_gallery(project, kind, cx);
        h_flex()
            .w_full()
            .items_start()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().group_box)
            .child(div().w(rems(22.0)).flex_shrink_0().p_4().child(gallery))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_4()
                    .p_5()
                    .child(
                        v_flex()
                            .w_full()
                            .min_w_0()
                            .gap_1()
                            .child(div().text_xl().font_semibold().child(project.title.clone()))
                            .when(!project.author.is_empty(), |this| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("作者：{}", project.author)),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .line_clamp(4)
                            .child(project.description.clone()),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .gap_4()
                            .flex_wrap()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{} 次下载", format_count(project.downloads)))
                            .when_some(project.updated.as_deref(), |this, updated| {
                                this.child(format!("更新于 {}", short_date(Some(updated))))
                            }),
                    )
                    .child(
                        v_flex()
                            .w_full()
                            .gap_2()
                            .rounded(cx.theme().radius)
                            .bg(cx.theme().primary.opacity(0.07))
                            .p_3()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("适配当前实例的推荐版本"),
                            )
                            .when_some(recommended_name, |this, name| {
                                this.child(div().text_sm().font_semibold().child(name))
                            })
                            .when_some(recommended_file, |this, file| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .truncate()
                                        .child(file),
                                )
                            })
                            .when(has_recommended, |this| {
                                this.child(
                                    Button::new("content-browser-install-recommended")
                                        .primary()
                                        .icon(IconName::Download)
                                        .label("安装推荐版本")
                                        .disabled(!can_install)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            if let Some(version) = install_version.clone() {
                                                this.install_remote_version(version, window, cx);
                                            }
                                        })),
                                )
                            })
                            .when(!has_recommended, |this| {
                                this.child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(if self.content_browser.versions_loading {
                                            "正在查找兼容版本…"
                                        } else {
                                            "当前实例没有可安装版本"
                                        }),
                                )
                            }),
                    )
                    .child(
                        Link::new("content-browser-modrinth-page")
                            .href(project.page_url())
                            .child(
                                h_flex()
                                    .gap_1()
                                    .text_sm()
                                    .child("在 Modrinth 查看项目")
                                    .child(Icon::new(IconName::ExternalLink).small()),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_project_gallery(
        &self,
        project: &RemoteProject,
        kind: ContentKind,
        cx: &App,
    ) -> AnyElement {
        let urls = project
            .gallery_previews()
            .into_iter()
            .take(6)
            .collect::<Vec<_>>();
        let Some(state) = self.content_browser.gallery.clone() else {
            let preview = project.preview_url().and_then(|url| {
                self.content_browser
                    .project_previews
                    .get(url)
                    .and_then(|path| path.as_deref())
            });
            return preview_canvas(preview, kind, true, cx);
        };
        let path_for = |url: &str| {
            self.content_browser
                .project_previews
                .get(url)
                .and_then(|path| path.as_deref())
        };
        Carousel::new(format!("content-project-gallery-{}", project.id), &state)
            .w_full()
            .accessibility_label(format!("{} 的项目截图", project.title))
            .child(
                CarouselContent::new(&state)
                    .w_full()
                    .children(urls.iter().enumerate().map(|(index, url)| {
                        CarouselItem::new(
                            format!("content-project-gallery-item-{url}"),
                            index,
                            &state,
                        )
                        .child(preview_canvas(
                            path_for(url),
                            kind,
                            true,
                            cx,
                        ))
                    })),
            )
            .when(urls.len() > 1, |this| {
                this.child(
                    CarouselPrevious::new(&state)
                        .with_size(Size::Small)
                        .accessibility_label("上一张截图"),
                )
                .child(
                    CarouselNext::new(&state)
                        .with_size(Size::Small)
                        .accessibility_label("下一张截图"),
                )
                .child(
                    CarouselPagination::new()
                        .accessibility_label("选择项目截图")
                        .children(urls.iter().enumerate().map(|(index, url)| {
                            CarouselPaginationItem::new(
                                format!("content-project-gallery-page-{url}"),
                                index,
                                &state,
                            )
                            .accessibility_label(format!("查看第 {} 张截图", index + 1))
                            .w_16()
                            .h_10()
                            .p_0()
                            .overflow_hidden()
                            .child(preview_thumbnail(
                                path_for(url),
                                kind,
                                cx,
                            ))
                        })),
                )
            })
            .into_any_element()
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
            .child(if is_visual_kind(kind) {
                self.render_visual_project_overview(project, kind, cx)
                    .into_any_element()
            } else {
                card(cx)
                    .child(
                        v_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_lg()
                                            .font_semibold()
                                            .child(project.title.clone()),
                                    )
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
                                        categories.into_iter().map(|category| {
                                            Tag::secondary().child(category).small()
                                        }),
                                    )
                                    .when_some(project.updated.as_deref(), |this, updated| {
                                        this.child(
                                            Tag::new()
                                                .outline()
                                                .child(format!(
                                                    "更新于 {}",
                                                    short_date(Some(updated))
                                                ))
                                                .small(),
                                        )
                                    }),
                            ),
                    )
                    .into_any_element()
            })
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
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.back_to_content_search(window, cx)
                            })),
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

fn load_more_button(loading: bool, cx: &mut Context<LauncherApp>) -> Button {
    Button::new("content-browser-load-more")
        .outline()
        .w_full()
        .label(if loading {
            "正在加载…"
        } else {
            "加载更多"
        })
        .disabled(loading)
        .on_click(cx.listener(|this, _, window, cx| this.load_remote_search(false, window, cx)))
}

fn is_visual_kind(kind: ContentKind) -> bool {
    matches!(kind, ContentKind::ResourcePack | ContentKind::ShaderPack)
}

fn preview_canvas(path: Option<&Path>, kind: ContentKind, detailed: bool, cx: &App) -> AnyElement {
    let canvas = div()
        .w_full()
        .rounded(cx.theme().radius)
        .overflow_hidden()
        .bg(cx.theme().muted.opacity(0.35));
    let canvas = if detailed {
        canvas.aspect_ratio(16. / 9.)
    } else {
        canvas.h_40()
    };
    canvas
        .when_some(path, |this, path| {
            this.child(img(path.to_path_buf()).size_full().object_fit(if detailed {
                ObjectFit::Contain
            } else {
                ObjectFit::Cover
            }))
        })
        .when(path.is_none(), |this| {
            this.flex().items_center().justify_center().child(
                Icon::new(content_icon(kind))
                    .with_size(Size::Large)
                    .text_color(cx.theme().muted_foreground),
            )
        })
        .into_any_element()
}

fn preview_thumbnail(path: Option<&Path>, kind: ContentKind, cx: &App) -> AnyElement {
    div()
        .size_full()
        .bg(cx.theme().muted.opacity(0.35))
        .when_some(path, |this, path| {
            this.child(
                img(path.to_path_buf())
                    .size_full()
                    .object_fit(ObjectFit::Cover),
            )
        })
        .when(path.is_none(), |this| {
            this.flex().items_center().justify_center().child(
                Icon::new(content_icon(kind))
                    .small()
                    .text_color(cx.theme().muted_foreground),
            )
        })
        .into_any_element()
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
