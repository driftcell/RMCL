use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use gpui_kit::{
    assets::IconName,
    base::{Selectable, StyledExt},
    component::{
        ActiveTheme, Disableable, Icon, Sizable, Size, WindowExt,
        button::{Button, ButtonVariant, ButtonVariants},
        h_flex,
        input::{Input, InputState},
        notification::Notification,
        scroll::ScrollableElement,
        spinner::Spinner,
        tag::Tag,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use rmcl_app::{
    MicrosoftAccountServiceError, PreparedSkin, SkinLibraryEntry, SkinSourceError, load_skin_file,
};
use rmcl_auth::{Account, MicrosoftAccount, MicrosoftAuthError, SkinVariant};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::super::{LauncherApp, Page};
use super::common::{
    card, empty_state, error_banner, field_label, page_header, section_title, warning_banner,
};

/// The draft skin currently being previewed.
pub struct SkinEditor {
    pub(crate) draft: Option<PreparedSkin>,
    pub(crate) variant: SkinVariant,
    pub(crate) _load_task: Option<Task<()>>,
}

impl Default for SkinEditor {
    fn default() -> Self {
        Self {
            draft: None,
            variant: SkinVariant::Classic,
            _load_task: None,
        }
    }
}

/// The link-import form and its mutually exclusive operation state.
pub struct LinkImport {
    pub(crate) input: Entity<InputState>,
    pub(crate) state: LinkImportState,
}

impl LinkImport {
    pub fn new(input: Entity<InputState>) -> Self {
        Self {
            input,
            state: LinkImportState::Idle,
        }
    }
}

pub enum LinkImportState {
    Idle,
    Loading { _task: Task<()> },
    Failed(String),
}

/// Skin mutation state. The task is dropped when the state changes.
pub enum ApplyState {
    Idle,
    Applying { _task: Task<()> },
    Cooldown { until: Instant, _task: Task<()> },
}

impl ApplyState {
    fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    fn is_applying(&self) -> bool {
        matches!(self, Self::Applying { .. })
    }

    fn cooldown_remaining(&self) -> Option<Duration> {
        match self {
            Self::Cooldown { until, .. } => Some(until.saturating_duration_since(Instant::now())),
            _ => None,
        }
    }
}

impl LauncherApp {
    pub(crate) fn render_skin(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        self.flush_pending_skin_profile(window, cx);
        self.sync_skin_viewer(cx);

        let content_width = (window.viewport_size().width - px(232.) - px(48.)).max(px(560.));
        let stage_width = if content_width < px(860.) { 400 } else { 500 };
        let stage_height = (stage_width as f32 * 1.24).round() as u32;
        self.skin_viewer.update(cx, |viewer, cx| {
            viewer.set_size(stage_width, stage_height, cx)
        });

        let account = self.selected_microsoft_account().cloned();
        let has_account = account.is_some();
        let refresh_view = cx.entity().downgrade();
        let refresh_button = Button::new("skin-refresh")
            .label("刷新")
            .outline()
            .small()
            .disabled(account.is_none())
            .on_click(move |_, window, cx| {
                let Some(view) = refresh_view.upgrade() else {
                    return;
                };
                view.update(cx, |view, cx| view.refresh_skin_profile(window, cx));
            });

        v_flex()
            .flex_1()
            .min_h_0()
            .p_6()
            .gap_6()
            .child(page_header(Page::Skin, refresh_button, cx))
            .when_some(self.config_warning.clone(), |this, warning| {
                this.child(error_banner("配置读取失败", &warning, cx))
            })
            .when_some(account, |this, account| {
                this.child(self.render_skin_content(account, content_width, cx))
            })
            .when(!has_account, |this| {
                let open_accounts = cx.entity().downgrade();
                this.child(empty_state(
                    IconName::Shirt,
                    "皮肤需要正版账户",
                    "离线账户没有官方皮肤。登录 Microsoft 账户后即可查看和更换。",
                    Button::new("skin-open-accounts")
                        .label("管理账户…")
                        .primary()
                        .on_click(move |_, window, cx| {
                            let Some(view) = open_accounts.upgrade() else {
                                return;
                            };
                            view.update(cx, |view, cx| view.open_accounts_dialog(window, cx));
                        }),
                    cx,
                ))
            })
    }

    fn render_skin_content(
        &self,
        account: MicrosoftAccount,
        _content_width: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let service = self.skin_service.read(cx);
        let profile_state = service.profile_state(account.uuid());
        let profile = service.profile(account.uuid());
        let texture_state = service.texture_state(account.uuid());
        let profile_error = profile_state.and_then(|state| state.error());
        let texture_error = texture_state.and_then(|state| state.error());
        let profile_loading = profile_state.is_some_and(|state| state.is_loading());

        h_flex()
            .items_stretch()
            .flex_1()
            .min_h_0()
            .gap_6()
            .child(self.skin_viewer.clone())
            .child(
                v_flex()
                    .id("skin-side")
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .items_stretch()
                    .overflow_y_scrollbar()
                    .child(
                        card(cx)
                            .child(section_title(
                                "当前皮肤",
                                "这里显示的是 Minecraft 服务中的激活皮肤。",
                                cx,
                            ))
                            .when_some(profile_error, |this, error| {
                                this.child(error_banner("无法同步皮肤", error, cx))
                            })
                            .when_some(texture_error, |this, error| {
                                this.child(error_banner("皮肤预览不可用", error, cx))
                            })
                            .child(self.render_profile_summary(account.clone(), profile, cx))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("skin-reset")
                                            .label("恢复默认…")
                                            .outline()
                                            .small()
                                            .disabled(
                                                profile.is_none()
                                                    || matches!(
                                                        self.skin_apply,
                                                        ApplyState::Applying { .. }
                                                    ),
                                            )
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.confirm_reset_skin(window, cx);
                                            })),
                                    )
                                    .when(profile_loading, |this| {
                                        this.child(
                                            h_flex()
                                                .gap_2()
                                                .child(
                                                    Spinner::new()
                                                        .with_size(Size::XSmall)
                                                        .color(cx.theme().primary),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(cx.theme().muted_foreground)
                                                        .child("正在同步官方皮肤…"),
                                                ),
                                        )
                                    }),
                            )
                            .child(div().h(px(1.)).bg(cx.theme().border))
                            .child(self.render_skin_editor(account.uuid(), cx))
                            .child(div().h(px(1.)).bg(cx.theme().border))
                            .child(self.render_recent_skins(cx)),
                    ),
            )
    }

    fn render_profile_summary(
        &self,
        account: MicrosoftAccount,
        profile: Option<&super::super::skin_service::LoadedProfile>,
        cx: &App,
    ) -> impl IntoElement {
        let profile = profile.map(|loaded| &loaded.profile).unwrap_or(&account);
        let account_uuid = profile.uuid();
        let head = self.account_heads.get(&account_uuid);
        let active_skin = profile.active_skin();
        let model = active_skin
            .map(|skin| variant_label(skin.variant()))
            .unwrap_or("默认模型");
        let skin_name = active_skin
            .and_then(|skin| skin.alias())
            .filter(|alias| !alias.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if active_skin.is_some() {
                    "官方皮肤".to_owned()
                } else {
                    "默认皮肤".to_owned()
                }
            });
        let cape = profile
            .capes()
            .iter()
            .find(|cape| cape.is_active())
            .and_then(|cape| cape.alias())
            .filter(|alias| !alias.is_empty());

        let avatar_account = Account::Microsoft(profile.clone());
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_3()
                    .child(super::super::account_avatar(
                        Some(&avatar_account),
                        head,
                        40.,
                        cx,
                    ))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_lg()
                                    .font_semibold()
                                    .truncate()
                                    .child(profile.username().to_owned()),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("{skin_name} · {model}")),
                            ),
                    ),
            )
            .when_some(cape, |this, cape| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("披风：{cape}")),
                )
            })
    }

    fn render_skin_editor(&self, account: Uuid, cx: &mut Context<Self>) -> impl IntoElement {
        let applying = self.skin_apply.is_applying();
        let cooldown = self.skin_apply.cooldown_remaining();
        let draft = self.skin_editor.draft.clone();

        v_flex()
            .gap_4()
            .child(section_title(
                "更换皮肤",
                "上传本地 PNG，或粘贴皮肤站的图片直链。",
                cx,
            ))
            .when_some(draft, |this, draft| {
                this.child(
                    v_flex()
                        .gap_3()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Icon::new(IconName::Shirt)
                                        .small()
                                        .text_color(cx.theme().primary),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .font_medium()
                                        .child(draft.file_name().to_owned()),
                                )
                                .child(
                                    Tag::info()
                                        .small()
                                        .child(variant_label(self.skin_editor.variant)),
                                ),
                        )
                        .when_some(draft.source_url().map(str::to_owned), |this, source| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .truncate()
                                    .child(format!("来源：{source}")),
                            )
                        })
                        .child(
                            h_flex()
                                .gap_2()
                                .child(field_label("模型"))
                                .child(
                                    Button::new("skin-model-classic")
                                        .label("经典")
                                        .outline()
                                        .selected(self.skin_editor.variant == SkinVariant::Classic)
                                        .small()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.skin_editor.variant = SkinVariant::Classic;
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("skin-model-slim")
                                        .label("纤细")
                                        .outline()
                                        .selected(self.skin_editor.variant == SkinVariant::Slim)
                                        .small()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.skin_editor.variant = SkinVariant::Slim;
                                            cx.notify();
                                        })),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("skin-apply")
                                        .label(if applying {
                                            "正在应用…"
                                        } else if cooldown.is_some() {
                                            "冷却中"
                                        } else {
                                            "应用皮肤"
                                        })
                                        .primary()
                                        .disabled(!self.skin_apply.is_idle())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.apply_skin(window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("skin-cancel-draft")
                                        .label("取消")
                                        .ghost()
                                        .disabled(applying)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.cancel_skin_draft(cx);
                                        })),
                                )
                                .when_some(cooldown, |this, remaining| {
                                    this.child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!(
                                                "{} 秒后可再次更换",
                                                remaining.as_secs().max(1)
                                            )),
                                    )
                                }),
                        ),
                )
            })
            .when(self.skin_editor.draft.is_none(), |this| {
                this.child(self.render_skin_import(account, cx))
            })
    }

    fn render_skin_import(&self, _account: Uuid, cx: &mut Context<Self>) -> impl IntoElement {
        let link_loading = matches!(self.skin_link.state, LinkImportState::Loading { .. });
        let link_error = match &self.skin_link.state {
            LinkImportState::Failed(error) => Some(error.clone()),
            _ => None,
        };
        let pick_view = cx.entity().downgrade();
        let read_view = cx.entity().downgrade();

        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("skin-pick-file")
                            .label("上传 PNG…")
                            .outline()
                            .disabled(link_loading)
                            .on_click(move |_, window, cx| {
                                let Some(view) = pick_view.upgrade() else {
                                    return;
                                };
                                view.update(cx, |view, cx| view.pick_skin_file(window, cx));
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("64×64 或旧版 64×32，最大 24 KiB"),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(field_label("图片链接"))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Input::new(&self.skin_link.input).w_full())
                            .child(
                                Button::new("skin-read-link")
                                    .label(if link_loading {
                                        "读取中…"
                                    } else {
                                        "读取"
                                    })
                                    .outline()
                                    .disabled(link_loading)
                                    .on_click(move |_, window, cx| {
                                        let Some(view) = read_view.upgrade() else {
                                            return;
                                        };
                                        view.update(cx, |view, cx| {
                                            view.fetch_skin_link(window, cx)
                                        });
                                    }),
                            ),
                    )
                    .when_some(link_error, |this, error| {
                        this.child(warning_banner("无法读取链接", &error, cx))
                    })
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("请粘贴 PNG 图片直链，而不是皮肤站网页地址。"),
                    ),
            )
    }

    fn render_recent_skins(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let recent = self.skin_service.read(cx).recent();
        let loading = recent.is_loading();
        let error = recent.error().map(str::to_owned);
        let entries = recent.value().cloned().unwrap_or_default();
        let retry_view = cx.entity().downgrade();
        let rows = entries
            .clone()
            .into_iter()
            .map(|entry| self.render_recent_row(entry, cx).into_any_element())
            .collect::<Vec<AnyElement>>();

        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .justify_between()
                    .child(section_title(
                        "最近使用",
                        "保存在本机的皮肤，方便再次应用。",
                        cx,
                    ))
                    .when(loading, |this| {
                        this.child(
                            Spinner::new()
                                .with_size(Size::XSmall)
                                .color(cx.theme().primary),
                        )
                    })
                    .when_some(error.clone(), |this, _| {
                        this.child(
                            Button::new("skin-recent-retry")
                                .label("重试")
                                .ghost()
                                .small()
                                .on_click(move |_, _, cx| {
                                    let Some(view) = retry_view.upgrade() else {
                                        return;
                                    };
                                    view.update(cx, |view, cx| {
                                        view.skin_service
                                            .update(cx, |service, cx| service.reload_recent(cx));
                                    });
                                }),
                        )
                    }),
            )
            .when_some(error, |this, error| {
                this.child(warning_banner("无法读取最近皮肤", &error, cx))
            })
            .when(entries.is_empty() && !loading, |this| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("还没有保存过皮肤。成功应用一次后，它会出现在这里。"),
                )
            })
            .child(v_flex().gap_2().children(rows))
    }

    fn render_recent_row(
        &self,
        entry: SkinLibraryEntry,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let head_path = self.services.skin_library.head_path(&entry.id);
        let use_view = cx.entity().downgrade();
        let entry_id = entry.id.clone();
        h_flex()
            .gap_3()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .p_2()
            .child(
                h_flex()
                    .size_9()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .rounded(cx.theme().radius)
                    .bg(cx.theme().muted)
                    .when_some(head_path.clone(), |this, path| {
                        this.child(
                            img(path)
                                .size_full()
                                .object_fit(ObjectFit::Cover)
                                .rounded(cx.theme().radius),
                        )
                    })
                    .when(head_path.is_none(), |this| {
                        this.child(
                            Icon::new(IconName::Shirt)
                                .small()
                                .text_color(cx.theme().muted_foreground),
                        )
                    }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_medium()
                            .truncate()
                            .child(entry.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(variant_label(entry.variant)),
                    ),
            )
            .child(
                Button::new(format!("skin-recent-use-{}", entry.id))
                    .label("使用")
                    .outline()
                    .small()
                    .on_click(move |_, window, cx| {
                        let Some(view) = use_view.upgrade() else {
                            return;
                        };
                        view.update(cx, |view, cx| {
                            view.use_recent_skin(entry_id.clone(), window, cx)
                        });
                    }),
            )
    }

    pub(crate) fn ensure_skin_page(&mut self, cx: &mut Context<Self>) {
        let Some(account) = self.selected_microsoft_account().cloned() else {
            self.skin_viewer.update(cx, |viewer, cx| viewer.clear(cx));
            return;
        };
        self.skin_service.update(cx, |service, cx| {
            service.ensure_profile(account, cx);
            service.ensure_recent(cx);
        });
    }

    fn flush_pending_skin_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(profile) = self.pending_skin_profile.take() else {
            return;
        };
        cx.defer_in(window, move |view, window, cx| {
            view.apply_microsoft_profile(profile, window, cx);
        });
    }

    fn sync_skin_viewer(&mut self, cx: &mut Context<Self>) {
        if let Some(draft) = self.skin_editor.draft.clone() {
            let key = format!("draft:{}", draft.id());
            let texture = draft.texture().clone();
            let variant = self.skin_editor.variant;
            self.skin_viewer.update(cx, |viewer, cx| {
                viewer.set_skin(key, texture, variant, true, cx);
            });
            return;
        }

        let Some(account) = self.selected_microsoft_account().cloned() else {
            self.skin_viewer.update(cx, |viewer, cx| viewer.clear(cx));
            return;
        };
        let equipped = self
            .skin_service
            .read(cx)
            .equipped_skin(account.uuid())
            .cloned();
        if let Some(skin) = equipped {
            let key = skin
                .url
                .as_deref()
                .map(|url| format!("url:{url}"))
                .unwrap_or_else(|| format!("default:{}", account.uuid()));
            let texture = skin.texture;
            let variant = skin.variant;
            self.skin_viewer.update(cx, |viewer, cx| {
                viewer.set_skin(key, texture, variant, false, cx);
            });
        } else {
            self.skin_viewer.update(cx, |viewer, cx| viewer.clear(cx));
        }
    }

    fn refresh_skin_profile(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(account) = self.selected_microsoft_account().cloned() else {
            return;
        };
        self.skin_service
            .update(cx, |service, cx| service.refresh_profile(account, cx));
    }

    fn pick_skin_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = self
            .services
            .config_store
            .path()
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();

        #[cfg(target_os = "macos")]
        {
            match pick_skin_file_macos(&start) {
                Ok(Some(path)) => self.load_skin_path(path, window, cx),
                Ok(None) => {}
                Err(error) => {
                    window.push_notification(Notification::error(error).title("选择皮肤失败"), cx);
                }
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let prompt = cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("选择 PNG 皮肤".into()),
            });
            self._prompt_task = Some(cx.spawn_in(
                window,
                async move |this, cx| match prompt.await {
                    Ok(Ok(Some(mut paths))) if !paths.is_empty() => {
                        let path = paths.remove(0);
                        let _ = this.update_in(cx, |view, window, cx| {
                            view.load_skin_path(path, window, cx)
                        });
                    }
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => {
                        let _ = this.update_in(cx, |_, window, cx| {
                            window.push_notification(
                                Notification::error(format!("无法打开文件选择器：{error}"))
                                    .title("选择皮肤失败"),
                                cx,
                            );
                        });
                    }
                    Err(_) => {}
                },
            ));
            cx.notify();
        }
    }

    fn load_skin_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("png"))
        {
            window.push_notification(
                Notification::warning("请选择 PNG 文件。").title("无法导入皮肤"),
                cx,
            );
            return;
        }
        let services = self.services.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let path_for_task = path.clone();
            let result = services
                .runtime
                .spawn(async move { load_skin_file(path_for_task).await })
                .await;
            let _ = this.update_in(cx, |view, window, cx| match result {
                Ok(Ok(skin)) => view.set_skin_draft(skin, cx),
                Ok(Err(error)) => {
                    window.push_notification(
                        Notification::error(describe_skin_source_error(&error))
                            .title("无法导入皮肤"),
                        cx,
                    );
                }
                Err(error) => {
                    window.push_notification(
                        Notification::error(format!("皮肤处理任务失败：{error}"))
                            .title("无法导入皮肤"),
                        cx,
                    );
                }
            });
        });
        self.skin_editor._load_task = Some(task);
        cx.notify();
    }

    fn fetch_skin_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.skin_link.state, LinkImportState::Loading { .. }) {
            return;
        }
        let url = self.skin_link.input.read(cx).value().trim().to_owned();
        if url.is_empty() {
            self.skin_link.state = LinkImportState::Failed("请输入图片直链。".to_owned());
            cx.notify();
            return;
        }
        let client = self.services.skin_urls.clone();
        let runtime = self.services.runtime.handle().clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let cancellation = CancellationToken::new();
            let url_for_task = url.clone();
            let result = runtime
                .spawn(async move { client.fetch(&url_for_task, &cancellation).await })
                .await;
            let _ = this.update_in(cx, |view, _window, cx| {
                if !matches!(view.skin_link.state, LinkImportState::Loading { .. }) {
                    return;
                }
                match result {
                    Ok(Ok(skin)) => {
                        view.skin_link.state = LinkImportState::Idle;
                        view.set_skin_draft(skin, cx);
                    }
                    Ok(Err(error)) => {
                        view.skin_link.state =
                            LinkImportState::Failed(describe_skin_source_error(&error));
                    }
                    Err(error) => {
                        view.skin_link.state =
                            LinkImportState::Failed(format!("读取任务失败：{error}"));
                    }
                }
                cx.notify();
            });
        });
        self.skin_link.state = LinkImportState::Loading { _task: task };
        cx.notify();
    }

    fn set_skin_draft(&mut self, skin: PreparedSkin, cx: &mut Context<Self>) {
        self.skin_editor.variant = skin.variant_hint();
        self.skin_editor.draft = Some(skin);
        self.skin_viewer
            .update(cx, |viewer, cx| viewer.reset_view(cx));
        cx.notify();
    }

    fn cancel_skin_draft(&mut self, cx: &mut Context<Self>) {
        self.skin_editor.draft = None;
        self.skin_viewer
            .update(cx, |viewer, cx| viewer.reset_view(cx));
        cx.notify();
    }

    fn use_recent_skin(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let library = self.services.skin_library.clone();
        let runtime = self.services.runtime.handle().clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = runtime.spawn(async move { library.load(&id).await }).await;
            let _ = this.update_in(cx, |view, window, cx| match result {
                Ok(Ok(skin)) => view.set_skin_draft(skin, cx),
                Ok(Err(error)) => {
                    window.push_notification(
                        Notification::error(format!("无法读取本地皮肤：{error}"))
                            .title("无法使用皮肤"),
                        cx,
                    );
                }
                Err(error) => {
                    window.push_notification(
                        Notification::error(format!("本地皮肤任务失败：{error}"))
                            .title("无法使用皮肤"),
                        cx,
                    );
                }
            });
        });
        self.skin_editor._load_task = Some(task);
        cx.notify();
    }

    fn apply_skin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.skin_apply.is_idle() {
            return;
        }
        let Some(account) = self.selected_microsoft_account().cloned() else {
            window.push_notification(
                Notification::warning("请先选择一个正版账户。").title("无法应用皮肤"),
                cx,
            );
            return;
        };
        let Some(draft) = self.skin_editor.draft.clone() else {
            return;
        };
        let variant = self.skin_editor.variant;
        let account_uuid = account.uuid();
        let accounts = self.services.microsoft_accounts.clone();
        let library = self.services.skin_library.clone();
        let runtime = self.services.runtime.handle().clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let upload_account = account.clone();
            let upload_draft = draft.clone();
            let result = runtime
                .spawn(async move {
                    accounts
                        .upload_skin(
                            &upload_account,
                            variant,
                            upload_draft.file_name(),
                            upload_draft.bytes(),
                            &CancellationToken::new(),
                        )
                        .await
                })
                .await;
            let stored = if matches!(&result, Ok(Ok(_))) {
                let draft = draft.clone();
                runtime
                    .spawn(async move { library.import(&draft).await })
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .is_some()
            } else {
                false
            };
            let _ = this.update_in(cx, move |view, window, cx| {
                if !matches!(view.skin_apply, ApplyState::Applying { .. })
                    || view
                        .selected_microsoft_account()
                        .map(MicrosoftAccount::uuid)
                        != Some(account_uuid)
                {
                    view.skin_apply = ApplyState::Idle;
                    cx.notify();
                    return;
                }
                match result {
                    Ok(Ok(profile)) => {
                        view.apply_microsoft_profile(profile.clone(), window, cx);
                        view.skin_service.update(cx, |service, cx| {
                            service.accept_profile(
                                profile,
                                Some((draft.texture().clone(), variant)),
                                cx,
                            );
                        });
                        if stored {
                            view.skin_service
                                .update(cx, |service, cx| service.reload_recent(cx));
                        }
                        view.skin_editor.draft = None;
                        view.skin_editor.variant = variant;
                        view.start_skin_cooldown(Duration::from_secs(2), cx);
                        window.push_notification(
                            Notification::success("皮肤已更换，重新进入游戏后生效。")
                                .title("皮肤已应用"),
                            cx,
                        );
                    }
                    Ok(Err(error)) => {
                        let (message, cooldown) = describe_apply_error(&error);
                        if let Some(cooldown) = cooldown {
                            view.start_skin_cooldown(cooldown, cx);
                        } else {
                            view.skin_apply = ApplyState::Idle;
                        }
                        window.push_notification(
                            Notification::error(message).title("更换皮肤失败"),
                            cx,
                        );
                    }
                    Err(error) => {
                        view.skin_apply = ApplyState::Idle;
                        window.push_notification(
                            Notification::error(format!("皮肤上传任务失败：{error}"))
                                .title("更换皮肤失败"),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        });
        self.skin_apply = ApplyState::Applying { _task: task };
        cx.notify();
    }

    fn confirm_reset_skin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .confirm()
                .title("恢复默认皮肤？")
                .description("当前皮肤会被移除，账户将使用默认外观。之后仍可重新上传。")
                .ok_text("恢复默认")
                .ok_variant(ButtonVariant::Danger)
                .on_ok({
                    let view = view.clone();
                    move |_, window, cx| {
                        let Some(view) = view.upgrade() else {
                            return true;
                        };
                        view.update(cx, |view, cx| view.reset_skin(window, cx));
                        true
                    }
                })
        });
    }

    fn reset_skin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.skin_apply.is_idle() {
            return;
        }
        let Some(account) = self.selected_microsoft_account().cloned() else {
            return;
        };
        let account_uuid = account.uuid();
        let accounts = self.services.microsoft_accounts.clone();
        let runtime = self.services.runtime.handle().clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let reset_account = account.clone();
            let result = runtime
                .spawn(async move {
                    accounts
                        .reset_skin(&reset_account, &CancellationToken::new())
                        .await
                })
                .await;
            let _ = this.update_in(cx, move |view, window, cx| {
                if !matches!(view.skin_apply, ApplyState::Applying { .. })
                    || view
                        .selected_microsoft_account()
                        .map(MicrosoftAccount::uuid)
                        != Some(account_uuid)
                {
                    view.skin_apply = ApplyState::Idle;
                    cx.notify();
                    return;
                }
                match result {
                    Ok(Ok(Some(profile))) => {
                        view.apply_microsoft_profile(profile.clone(), window, cx);
                        view.skin_service.update(cx, |service, cx| {
                            service.accept_profile(profile, None, cx);
                        });
                        view.skin_editor.draft = None;
                        view.start_skin_cooldown(Duration::from_secs(2), cx);
                        window.push_notification(
                            Notification::success("已恢复默认皮肤，重新进入游戏后生效。")
                                .title("皮肤已恢复"),
                            cx,
                        );
                    }
                    Ok(Ok(None)) => {
                        view.skin_service.update(cx, |service, cx| {
                            service.refresh_profile(account, cx);
                        });
                        view.start_skin_cooldown(Duration::from_secs(2), cx);
                        window.push_notification(
                            Notification::info("已请求恢复默认皮肤，正在同步结果。")
                                .title("皮肤已恢复"),
                            cx,
                        );
                    }
                    Ok(Err(error)) => {
                        let (message, cooldown) = describe_apply_error(&error);
                        if let Some(cooldown) = cooldown {
                            view.start_skin_cooldown(cooldown, cx);
                        } else {
                            view.skin_apply = ApplyState::Idle;
                        }
                        window.push_notification(
                            Notification::error(message).title("恢复默认皮肤失败"),
                            cx,
                        );
                    }
                    Err(error) => {
                        view.skin_apply = ApplyState::Idle;
                        window.push_notification(
                            Notification::error(format!("皮肤重置任务失败：{error}"))
                                .title("恢复默认皮肤失败"),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        });
        self.skin_apply = ApplyState::Applying { _task: task };
        cx.notify();
    }

    fn start_skin_cooldown(&mut self, duration: Duration, cx: &mut Context<Self>) {
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(duration).await;
            let _ = this.update(cx, |view, cx| {
                if matches!(view.skin_apply, ApplyState::Cooldown { .. }) {
                    view.skin_apply = ApplyState::Idle;
                    cx.notify();
                }
            });
        });
        self.skin_apply = ApplyState::Cooldown {
            until: Instant::now() + duration,
            _task: task,
        };
        cx.notify();
    }

    fn selected_microsoft_account(&self) -> Option<&MicrosoftAccount> {
        self.selected_account().and_then(Account::as_microsoft)
    }
}

fn variant_label(variant: SkinVariant) -> &'static str {
    match variant {
        SkinVariant::Classic => "经典模型",
        SkinVariant::Slim => "纤细模型",
    }
}

fn describe_apply_error(error: &MicrosoftAccountServiceError) -> (String, Option<Duration>) {
    match error {
        MicrosoftAccountServiceError::CredentialsMissing
        | MicrosoftAccountServiceError::Auth(
            MicrosoftAuthError::ReauthenticationRequired
            | MicrosoftAuthError::MinecraftTokenRejected,
        ) => ("登录已过期，请重新登录后再试。".to_owned(), None),
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::SkinRateLimited { retry_after }) => {
            (
                "官方限制了频繁更换皮肤，请稍后再试。".to_owned(),
                Some(Duration::from_secs(retry_after.unwrap_or(30))),
            )
        }
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::SkinRejected { message }) => {
            (format!("官方拒绝了这张皮肤：{message}"), None)
        }
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::SkinForbidden { .. }) => {
            ("当前账户暂时被限制更换皮肤。".to_owned(), None)
        }
        MicrosoftAccountServiceError::Auth(
            MicrosoftAuthError::Http { .. } | MicrosoftAuthError::HttpStatus { .. },
        ) => (
            "网络错误，请稍后重试。若请求可能已发出，请先刷新当前皮肤确认结果。".to_owned(),
            None,
        ),
        other => (other.to_string(), None),
    }
}

fn describe_skin_source_error(error: &SkinSourceError) -> String {
    match error {
        SkinSourceError::UnsupportedScheme | SkinSourceError::CredentialsNotAllowed => {
            "请使用 HTTPS 图片直链。".to_owned()
        }
        SkinSourceError::PrivateAddress => "该链接指向本地或私有地址，无法读取。".to_owned(),
        SkinSourceError::NotPng => "该链接不是 PNG 图片，请使用图片直链。".to_owned(),
        SkinSourceError::HttpStatus { status } => {
            format!("读取图片失败（HTTP {status}）。")
        }
        SkinSourceError::SourceTooLarge => "图片超过 1 MiB，无法读取。".to_owned(),
        SkinSourceError::UnsupportedUploadDimensions { width, height } => {
            format!("Java 版官方皮肤必须是 64×64 或 64×32，这张图片是 {width}×{height}。")
        }
        SkinSourceError::UploadTooLarge => {
            "图片超过官方 24 KiB 上传限制，请压缩后再试。".to_owned()
        }
        SkinSourceError::Skin(_) => "图片不是有效的 PNG 皮肤。".to_owned(),
        SkinSourceError::InvalidUrl(_) | SkinSourceError::MissingHost => {
            "图片链接格式无效。".to_owned()
        }
        SkinSourceError::Http(_) | SkinSourceError::Resolve(_) => {
            "无法连接图片地址，请检查网络或改用本地文件。".to_owned()
        }
        SkinSourceError::Cancelled => "读取已取消。".to_owned(),
        SkinSourceError::TooManyRedirects | SkinSourceError::MissingLocation => {
            "图片链接重定向过多或无效。".to_owned()
        }
        SkinSourceError::Encode(_) => "无法处理这张 PNG 图片。".to_owned(),
        SkinSourceError::Io { .. } => "无法读取本地文件。".to_owned(),
    }
}

#[cfg(target_os = "macos")]
fn pick_skin_file_macos(start: &Path) -> Result<Option<PathBuf>, String> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
    use objc2_foundation::{NSString, NSURL};

    let marker =
        MainThreadMarker::new().ok_or_else(|| "文件选择器必须在主线程打开。".to_owned())?;
    let panel = NSOpenPanel::openPanel(marker);
    panel.setCanChooseDirectories(false);
    panel.setCanChooseFiles(true);
    panel.setAllowsMultipleSelection(false);
    panel.setResolvesAliases(false);
    if start.is_dir()
        && let Some(url) = NSURL::from_directory_path(start)
    {
        panel.setDirectoryURL(Some(&url));
    }
    panel.setPrompt(Some(&NSString::from_str("选择")));
    if panel.runModal() != NSModalResponseOK {
        return Ok(None);
    }
    Ok(panel.URLs().iter().find_map(|url| url.to_file_path()))
}
