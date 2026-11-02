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
use crate::i18n::tr;
use rmcl_store::DownloadSourcePreference;

impl LauncherApp {
    pub fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let agent_keyless = !self.agent.provider(cx).requires_key();
        let auto_java_view = cx.entity().downgrade();
        let auto_update_view = cx.entity().downgrade();
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
                                .child(
                                    rust_i18n::t!(
                                        "app.settings.config_save_failed",
                                        error = error
                                    )
                                    .to_string(),
                                ),
                        )
                    })
                    .child(
                        card(cx)
                            .child(section_title(tr("app.settings.game_section"), "", cx))
                            .child(
                                v_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(field_label(tr("app.settings.minecraft_dir")))
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
                                                    .label(tr("app.settings.choose_directory"))
                                                    .disabled(!self.log_state.can_start())
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            this.browse_minecraft_root(window, cx)
                                                        },
                                                    )),
                                            ),
                                    )
                                    .child(help(tr("app.settings.minecraft_dir_help"), cx)),
                            )
                            .child(
                                v_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(field_label(tr("app.settings.java_dir")))
                                    .child(mono(display_path(&self.config.java_store), cx))
                                    .child(help(tr("app.settings.restart_required"), cx)),
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
                                            .child(field_label(tr("app.settings.auto_java")))
                                            .child(help(
                                                tr("app.settings.auto_java_help"),
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
                                tr("app.settings.download_section"),
                                tr("app.settings.download_section_subtitle"),
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
                                        .child(field_label(tr("app.settings.download_source")))
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(tr("app.settings.download_source_help")),
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
                                                .child(tr("app.settings.mirror_provided_by"))
                                                .child(
                                                    Link::new("bmclapi-attribution")
                                                        .href("https://bmclapidoc.bangbang93.com/")
                                                        .child("BMCLAPI"),
                                                )
                                                .child(tr("app.settings.mirror_provided_by_suffix")),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .flex_shrink_0()
                                        .child(
                                            div().w(rems(11.25)).child(
                                                Select::new(&self.download_source_select)
                                                    .accessibility_label(tr("app.settings.download_source"))
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
                                                .tooltip(tr("app.settings.probe_mirror"))
                                                .accessibility_label(tr("app.settings.probe_mirror"))
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
                                tr("app.settings.launch_behavior"),
                                tr("app.settings.launch_behavior_subtitle"),
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
                                            .child(field_label(tr("app.settings.launcher_visibility")))
                                            .child(help(tr("app.settings.launcher_visibility_help"), cx)),
                                    )
                                    .child(
                                        div().flex_shrink_0().w(rems(11.25)).child(
                                            Select::new(&self.visibility_select)
                                                .accessibility_label(tr("app.settings.launcher_visibility"))
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
                                tr("app.settings.ai_section"),
                                tr("app.settings.ai_section_subtitle"),
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
                                            .child(field_label(tr("app.settings.provider")))
                                            .child(help(tr("app.settings.provider_help"), cx)),
                                    )
                                    .child(
                                        div().flex_shrink_0().w(rems(13.5)).child(
                                            Select::new(&self.agent.settings.provider_select)
                                                .accessibility_label(tr("app.settings.ai_provider"))
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
                                            .child(field_label(tr("app.settings.model_id")))
                                            .child(Input::new(&self.agent.settings.model_input).w_full()),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(field_label(if agent_keyless {
                                                tr("app.settings.free_tier")
                                            } else {
                                                "API Key".to_owned()
                                            }))
                                            .child(if agent_keyless {
                                                h_flex()
                                                    .gap_2()
                                                    .items_center()
                                                    .child(
                                                        Tag::success()
                                                            .small()
                                                            .child(tr("app.settings.no_key")),
                                                    )
                                                    .child(help(
                                                        tr("app.settings.free_tier_warning"),
                                                        cx,
                                                    ))
                                                    .into_any_element()
                                            } else {
                                                match &self.agent.settings.key {
                                                    AgentKeyState::Saved => h_flex()
                                                        .gap_2()
                                                        .child(help(tr("app.settings.key_saved"), cx))
                                                        .child(
                                                            Button::new(
                                                                "settings-agent-remove-key",
                                                            )
                                                            .ghost()
                                                            .small()
                                                            .label(tr("app.common.remove"))
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
                                                            .label(tr("app.common.save"))
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
                                                            .label(tr("app.common.save"))
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
                                            .child(field_label(tr("app.settings.proxy_optional")))
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
                                            .child(tr("app.settings.agent_data_notice")),
                                    )
                                    .child(
                                        Button::new("settings-agent-save")
                                            .primary()
                                            .label(tr("app.settings.save_ai"))
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
                                tr("app.settings.appearance_title"),
                                tr("app.settings.appearance_subtitle"),
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
                                            .child(field_label(tr("app.settings.theme")))
                                            .child(help(tr("app.settings.theme_help"), cx)),
                                    )
                                    .child(
                                        div().flex_shrink_0().w(rems(11.25)).child(
                                            Select::new(&self.theme_select)
                                                .accessibility_label(tr("app.settings.theme"))
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
                                            .child(field_label(tr("app.settings.accent")))
                                            .child(help(tr("app.settings.accent_help"), cx)),
                                    )
                                    .child(
                                        div().flex_shrink_0().w(rems(11.25)).child(
                                            Select::new(&self.accent_select)
                                                .accessibility_label(tr("app.settings.accent"))
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
                                            .child(field_label(tr("app.settings.language")))
                                            .child(help(tr("app.settings.language_help"), cx)),
                                    )
                                    .child(
                                        div().flex_shrink_0().w(rems(11.25)).child(
                                            Select::new(&self.language_select)
                                                .accessibility_label(tr("app.settings.language"))
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
                                tr("app.settings.updates_section"),
                                tr("app.settings.updates_section_subtitle"),
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
                                            .child(field_label(tr("app.settings.current_version")))
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
                                            .child(field_label(tr("app.settings.check_on_start")))
                                            .child(help(tr("app.settings.check_on_start_help"), cx)),
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
                                                tr("app.settings.checking")
                                            } else if update_up_to_date {
                                                tr("app.settings.up_to_date")
                                            } else {
                                                tr("app.settings.check_updates")
                                            })
                                            .loading(update_checking)
                                            .disabled(self.update.downloading)
                                            .tooltip(if update_up_to_date {
                                                tr("app.settings.recheck_updates")
                                            } else {
                                                tr("app.settings.check_updates")
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
                                                .label(
                                                    rust_i18n::t!(
                                                        "app.settings.update_to",
                                                        version = release.version
                                                    )
                                                    .to_string(),
                                                )
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
                                                .label(tr("app.settings.release_notes"))
                                                .on_click({
                                                    let url = release.page_url.to_string();
                                                    move |_, _, cx| cx.open_url(&url)
                                                }),
                                        )
                                    }),
                            )
                            .when_some(self.update.progress, |this, progress| {
                                let label = match progress.total {
                                    Some(total) if total > 0 => rust_i18n::t!(
                                        "app.settings.downloading_progress",
                                        percent = progress.downloaded as f64 / total as f64 * 100.0 : {:.0},
                                        downloaded = format_size(progress.downloaded),
                                        total = format_size(total)
                                    )
                                    .to_string(),
                                    _ => rust_i18n::t!(
                                        "app.settings.downloading",
                                        downloaded = format_size(progress.downloaded)
                                    )
                                    .to_string(),
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
                            .child(section_title(
                                tr("app.settings.about_section"),
                                tr("app.settings.about_section_subtitle"),
                                cx,
                            ))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(Tag::primary().child(env!("CARGO_PKG_VERSION")).small())
                                    .child(Tag::new().outline().child("MIT").small())
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(tr("app.settings.tagline")),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .w_full()
                                    .gap_1()
                                    .child(field_label(tr("app.settings.config_file")))
                                    .child(mono(
                                        display_path(self.services.config_store.path()),
                                        cx,
                                    )),
                            )
                            .child(
                                v_flex()
                                    .w_full()
                                    .gap_1()
                                    .child(field_label(tr("app.settings.platform")))
                                    .child(mono(
                                        rust_i18n::t!(
                                            "app.settings.platform_line",
                                            platform = super::super::platform_label(
                                                &self.services.platform
                                            )
                                        )
                                        .to_string(),
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
                                            .label(tr("app.settings.gpui_docs"))
                                            .on_click(|_, _, cx| {
                                                cx.open_url("https://gpui-kit.com");
                                            }),
                                    )
                                    .child(
                                        Button::new("settings-folder")
                                            .ghost()
                                            .small()
                                            .icon(IconName::FolderOpen)
                                            .label(tr("app.home.open_game_folder"))
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.open_game_folder(window, cx)
                                            })),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(tr("app.settings.disclaimer")),
                            ),
                    ),
            )
    }
}

fn mirror_status_color(status: &str, cx: &App) -> Hsla {
    if status.starts_with("国内镜像可用")
        || status.starts_with("国内镜像优先（")
        || status.starts_with("Domestic mirror available")
        || status.starts_with("Preferring the domestic mirror (")
    {
        cx.theme().success
    } else if status.contains("不可用")
        || status.contains("失败")
        || status.contains("unavailable")
        || status.contains("failed")
    {
        cx.theme().warning
    } else {
        cx.theme().muted_foreground
    }
}
