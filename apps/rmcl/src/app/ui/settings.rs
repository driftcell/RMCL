use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Sizable,
        button::{Button, ButtonVariants},
        h_flex,
        input::Input,
        link::Link,
        scroll::ScrollableElement,
        select::Select,
        switch::Switch,
        tag::Tag,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};

use super::super::{LauncherApp, Page, agent::AgentKeyState};
use super::common::{
    card, display_path, field_label, format_size, help, mono, page_header, section_title,
};
use super::layout;
use rmcl_store::DownloadSourcePreference;

impl LauncherApp {
    pub fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let agent_keyless = !self.agent.provider(cx).requires_key();
        let auto_java_view = cx.entity().downgrade();
        let auto_update_view = cx.entity().downgrade();
        let mcp_view = cx.entity().downgrade();
        let (mcp_status_text, mcp_status_color) = if self.mcp_pending {
            ("正在应用 MCP 设置…".to_owned(), cx.theme().muted_foreground)
        } else if self.mcp_status.running {
            (
                format!(
                    "运行中 · http://127.0.0.1:{}/mcp · 只读",
                    self.mcp_status.port
                ),
                cx.theme().success,
            )
        } else if self.mcp_status.enabled {
            ("已启用，服务未运行".to_owned(), cx.theme().warning)
        } else {
            ("已停用".to_owned(), cx.theme().muted_foreground)
        };
        let update_checking = self.update.checking;
        let update_up_to_date = self.update.up_to_date && !update_checking;
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
                    .w_full()
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
                            .child(section_title("游戏", "", cx))
                            .child(
                                v_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(field_label("Minecraft 目录"))
                                    .child(
                                        h_flex()
                                            .w_full()
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
                                                    .child(display_path(
                                                        &self.config.minecraft_root,
                                                    )),
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
                                    .child(help("更改后立即重新扫描版本，不移动现有文件。", cx)),
                            )
                            .child(
                                v_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(field_label("Java 目录"))
                                    .child(mono(display_path(&self.config.java_store), cx))
                                    .child(help("重启 RMCL 后生效。", cx)),
                            )
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_4()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(field_label("自动获取 Java"))
                                            .child(help(
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
                            .child(section_title("下载", "选择 Minecraft 文件的下载来源。", cx))
                            .child(
                            h_flex()
                                .w_full()
                                .gap_4()
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap_1()
                                        .child(field_label("下载源"))
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child("自动会先探测国内镜像，失败时使用官方源。"),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(mirror_status_color(
                                                    &self.mirror_status,
                                                    cx,
                                                ))
                                                .child(self.mirror_status.clone()),
                                        )
                                        .child(
                                            h_flex()
                                                .gap_1()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child("国内镜像由")
                                                .child(
                                                    Link::new("bmclapi-attribution")
                                                        .href("https://bmclapidoc.bangbang93.com/")
                                                        .child("BMCLAPI"),
                                                )
                                                .child("提供。"),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .flex_shrink_0()
                                        .child(
                                            div().w(rems(11.25)).child(
                                                Select::new(&self.download_source_select)
                                                    .accessibility_label("下载源")
                                                    .menu_width(layout::rem_px(
                                                        cx,
                                                        layout::SELECT_MENU_WIDTH,
                                                    ))
                                                    .menu_max_h(layout::rem_px(
                                                        cx,
                                                        layout::SELECT_MENU_WIDTH,
                                                    ))
                                                    .cleanable(false)
                                                    .w_full(),
                                            ),
                                        )
                                        .child(
                                            Button::new("settings-mirror-probe")
                                                .ghost()
                                                .xsmall()
                                                .icon(IconName::RefreshCw)
                                                .tooltip("重新探测国内镜像")
                                                .accessibility_label("重新探测国内镜像")
                                                .disabled(
                                                    self.config.download_source
                                                        == DownloadSourcePreference::Official,
                                                )
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.retry_mirror_probe(cx)
                                                })),
                                        ),
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
                                    .w_full()
                                    .gap_4()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(field_label("启动器可见性"))
                                            .child(help("游戏退出后自动恢复。", cx)),
                                    )
                                    .child(
                                        div().flex_shrink_0().w(rems(11.25)).child(
                                            Select::new(&self.visibility_select)
                                                .accessibility_label("启动器可见性")
                                                .menu_width(layout::rem_px(
                                                    cx,
                                                    layout::SELECT_MENU_WIDTH,
                                                ))
                                                .menu_max_h(layout::rem_px(
                                                    cx,
                                                    layout::SELECT_MENU_WIDTH,
                                                ))
                                                .cleanable(false)
                                                .w_full(),
                                        ),
                                    ),
                            ),
                    )
                    .child(
                        card(cx)
                            .child(section_title(
                                "AI 助手",
                                "可使用 Kilo 免费模型，无需 API Key；也可填写自己的 API Key。Key 只保存在系统钥匙串。",
                                cx,
                            ))
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_4()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(field_label("服务商"))
                                            .child(help("模型 ID 可编辑；默认值会随 RMCL 版本更新。", cx)),
                                    )
                                    .child(
                                        div().flex_shrink_0().w(rems(13.5)).child(
                                            Select::new(&self.agent.settings.provider_select)
                                                .accessibility_label("AI 服务商")
                                                .menu_width(layout::rem_px(cx, layout::MENU_WIDTH))
                                                .menu_max_h(layout::rem_px(cx, layout::MENU_MAX_HEIGHT))
                                                .cleanable(false)
                                                .w_full(),
                                        ),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(field_label("模型 ID"))
                                            .child(Input::new(&self.agent.settings.model_input).w_full()),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(field_label(if agent_keyless {
                                                "免费通道"
                                            } else {
                                                "API Key"
                                            }))
                                            .child(if agent_keyless {
                                                h_flex()
                                                    .gap_2()
                                                    .items_center()
                                                    .child(
                                                        Tag::success()
                                                            .small()
                                                            .child("无需 Key"),
                                                    )
                                                    .child(help(
                                                        "免费模型可能记录对话，请勿发送敏感信息。",
                                                        cx,
                                                    ))
                                                    .into_any_element()
                                            } else {
                                                match &self.agent.settings.key {
                                                    AgentKeyState::Saved => h_flex()
                                                        .gap_2()
                                                        .child(help("已保存在系统钥匙串。", cx))
                                                        .child(
                                                            Button::new(
                                                                "settings-agent-remove-key",
                                                            )
                                                            .ghost()
                                                            .small()
                                                            .label("移除")
                                                            .on_click(cx.listener(
                                                                |this, _, _, cx| {
                                                                    this.remove_agent_key(cx)
                                                                },
                                                            )),
                                                        )
                                                        .into_any_element(),
                                                    AgentKeyState::Unknown
                                                    | AgentKeyState::Missing
                                                    | AgentKeyState::Free => h_flex()
                                                        .gap_2()
                                                        .child(
                                                            Input::new(
                                                                &self.agent.settings.key_input,
                                                            )
                                                            .mask_toggle()
                                                            .disabled(matches!(
                                                                self.agent.settings.key,
                                                                AgentKeyState::Unknown
                                                            ))
                                                            .w_full(),
                                                        )
                                                        .child(
                                                            Button::new(
                                                                "settings-agent-save-key",
                                                            )
                                                            .outline()
                                                            .small()
                                                            .label("保存")
                                                            .disabled(matches!(
                                                                self.agent.settings.key,
                                                                AgentKeyState::Unknown
                                                            ))
                                                            .on_click(cx.listener(
                                                                |this, _, window, cx| {
                                                                    this.save_agent_key(window, cx)
                                                                },
                                                            )),
                                                        )
                                                        .into_any_element(),
                                                    AgentKeyState::Error(error) => v_flex()
                                                        .gap_1()
                                                        .child(
                                                            Input::new(
                                                                &self.agent.settings.key_input,
                                                            )
                                                            .mask_toggle()
                                                            .disabled(matches!(
                                                                self.agent.settings.key,
                                                                AgentKeyState::Unknown
                                                            ))
                                                            .w_full(),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_xs()
                                                                .text_color(cx.theme().danger)
                                                                .child(error.clone()),
                                                        )
                                                        .child(
                                                            Button::new(
                                                                "settings-agent-save-key",
                                                            )
                                                            .outline()
                                                            .small()
                                                            .label("保存")
                                                            .disabled(matches!(
                                                                self.agent.settings.key,
                                                                AgentKeyState::Unknown
                                                            ))
                                                            .on_click(cx.listener(
                                                                |this, _, window, cx| {
                                                                    this.save_agent_key(window, cx)
                                                                },
                                                            )),
                                                        )
                                                        .into_any_element(),
                                                }
                                            }),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(field_label("Base URL"))
                                            .child(Input::new(&self.agent.settings.base_url_input).w_full()),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(field_label("代理（可选）"))
                                            .child(Input::new(&self.agent.settings.proxy_input).w_full()),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("对话内容和工具结果（包括实例名、路径、玩家名）会发送给所选服务商。"),
                                    )
                                    .child(
                                        Button::new("settings-agent-save")
                                            .primary()
                                            .label("保存 AI 设置")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                if let Err(error) = this.save_agent_settings(cx) {
                                                    this.agent.settings.error = Some(error);
                                                }
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .when_some(self.agent.settings.error.clone(), |this, error| {
                                this.child(
                                    div().text_sm().text_color(cx.theme().danger).child(error),
                                )
                            }),
                    )
                    .child(
                        card(cx)
                            .child(section_title(
                                "MCP",
                                "供本机 MCP 客户端查询实例、账户、Java 和任务状态。",
                                cx,
                            ))
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_4()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(field_label("本地只读服务"))
                                            .child(help(
                                                "仅监听 127.0.0.1，不允许写操作；默认停用。",
                                                cx,
                                            )),
                                    )
                                    .child(
                                        Switch::new("settings-mcp-enabled")
                                            .flex_shrink_0()
                                            .checked(self.config.mcp_enabled)
                                            .disabled(self.mcp_pending)
                                            .on_change(move |value, window, cx| {
                                                let _ = mcp_view.update(cx, |this, cx| {
                                                    this.set_mcp_enabled(*value, window, cx)
                                                });
                                            }),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .w_full()
                                    .items_end()
                                    .gap_2()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(field_label("HTTP 端口"))
                                            .child(help("修改后重启本地服务。", cx)),
                                    )
                                    .child(
                                        div().flex_shrink_0().w(rems(8.)).child(
                                            Input::new(&self.mcp_port_input)
                                                .disabled(self.mcp_pending)
                                                .w_full(),
                                        ),
                                    )
                                    .child(
                                        Button::new("settings-mcp-apply")
                                            .outline()
                                            .label("应用")
                                            .disabled(self.mcp_pending)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.apply_mcp_port(window, cx)
                                            })),
                                    ),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(mcp_status_color)
                                    .child(mcp_status_text),
                            )
                            .when_some(self.mcp_status.error.clone(), |this, error| {
                                this.child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().danger)
                                        .child(error),
                                )
                            }),
                    )
                    .child(
                        card(cx)
                            .child(section_title("外观", "修改后立即生效。", cx))
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_4()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(field_label("主题"))
                                            .child(help("跟随系统、浅色或深色。", cx)),
                                    )
                                    .child(
                                        div().flex_shrink_0().w(rems(11.25)).child(
                                            Select::new(&self.theme_select)
                                                .accessibility_label("主题")
                                                .menu_width(layout::rem_px(
                                                    cx,
                                                    layout::SELECT_MENU_WIDTH,
                                                ))
                                                .menu_max_h(layout::rem_px(
                                                    cx,
                                                    layout::SELECT_MENU_WIDTH,
                                                ))
                                                .cleanable(false)
                                                .w_full(),
                                        ),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_4()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(field_label("强调色"))
                                            .child(help("主按钮和选中状态的颜色。", cx)),
                                    )
                                    .child(
                                        div().flex_shrink_0().w(rems(11.25)).child(
                                            Select::new(&self.accent_select)
                                                .accessibility_label("强调色")
                                                .menu_width(layout::rem_px(
                                                    cx,
                                                    layout::SELECT_MENU_WIDTH,
                                                ))
                                                .menu_max_h(layout::rem_px(
                                                    cx,
                                                    layout::SELECT_MENU_WIDTH,
                                                ))
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
                                    .w_full()
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
                                            .child(help("自动检查发现新版本后提示。", cx)),
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
                                            .icon(if update_up_to_date {
                                                IconName::CircleCheck
                                            } else {
                                                IconName::RefreshCw
                                            })
                                            .label(if update_checking {
                                                "正在检查"
                                            } else if update_up_to_date {
                                                "已是最新版本"
                                            } else {
                                                "检查更新"
                                            })
                                            .loading(update_checking)
                                            .disabled(self.update.downloading)
                                            .tooltip(if update_up_to_date {
                                                "重新检查更新"
                                            } else {
                                                "检查更新"
                                            })
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
                                                .link()
                                                .small()
                                                .icon(IconName::ExternalLink)
                                                .label("发布说明")
                                                .on_click({
                                                    let url = release.page_url.to_string();
                                                    move |_, _, cx| cx.open_url(&url)
                                                }),
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
                            .when_some(self.update.error.clone(), |this, error| {
                                this.child(
                                    div().text_sm().text_color(cx.theme().danger).child(error),
                                )
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
                                        display_path(self.services.config_store.path()),
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
                                            .link()
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

fn mirror_status_color(status: &str, cx: &App) -> Hsla {
    if status.starts_with("国内镜像可用") || status.starts_with("国内镜像优先（") {
        cx.theme().success
    } else if status.contains("不可用") || status.contains("失败") {
        cx.theme().warning
    } else {
        cx.theme().muted_foreground
    }
}
