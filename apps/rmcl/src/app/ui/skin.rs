use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use super::super::{LauncherApp, Page, SkinTab, skin_service::CatalogThumbState};
use super::common::{
    card, empty_state, error_banner, field_label, page_header, section_title, warning_banner,
};
use super::layout;
use crate::i18n::tr;
use gpui_kit::{
    assets::IconName,
    base::{Selectable, StyledExt},
    component::{
        ActiveTheme, Disableable, Icon, Sizable, Size, WindowExt,
        button::{Button, ButtonVariants},
        h_flex,
        input::{Input, InputState},
        notification::Notification,
        scroll::ScrollableElement,
        spinner::Spinner,
        tab::{Tab, TabBar},
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use rmcl_app::{
    MicrosoftAccountServiceError, PreparedSkin, SkinCatalogItem, SkinCatalogSort, SkinLibraryEntry,
    SkinSourceError, load_skin_file,
};
use rmcl_auth::{Account, MicrosoftAccount, MicrosoftAuthError, SkinVariant};
use tokio_util::sync::CancellationToken;

/// The draft skin currently being previewed.
pub struct SkinEditor {
    pub(crate) draft: Option<PreparedSkin>,
    pub(crate) catalog_item: Option<SkinCatalogItem>,
    pub(crate) variant: SkinVariant,
    pub(crate) _load_task: Option<Task<()>>,
}

impl Default for SkinEditor {
    fn default() -> Self {
        Self {
            draft: None,
            catalog_item: None,
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
        self.flush_pending_catalog_skin(cx);
        self.sync_skin_viewer(cx);

        let viewport_width = window.viewport_size().width;
        let content_width = viewport_width
            - layout::rem_px(cx, layout::SIDEBAR_WIDTH)
            - layout::rem_px(cx, rems(3.0));
        let stage_width = (content_width * 0.46).clamp(
            layout::rem_px(cx, layout::SKIN_STAGE_MIN),
            layout::rem_px(cx, layout::SKIN_STAGE_MAX),
        );
        self.skin_viewer.update(cx, |viewer, cx| {
            viewer.set_width(stage_width.as_f32().round() as u32, cx)
        });

        let account = self.selected_microsoft_account().cloned();
        let selected_is_authlib = matches!(self.selected_account(), Some(Account::Authlib(_)));
        let selected_is_offline = matches!(self.selected_account(), Some(Account::Offline(_)));
        let refresh_view = cx.entity().downgrade();
        let refresh_button = Button::new("skin-refresh")
            .label(tr("app.skin.refresh_profile"))
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
                this.child(error_banner(tr("app.home.config_read_failed"), warning, cx))
            })
            .child(self.render_skin_content(account, selected_is_authlib, selected_is_offline, cx))
    }

    fn render_skin_content(
        &self,
        account: Option<MicrosoftAccount>,
        selected_is_authlib: bool,
        selected_is_offline: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let service = self.skin_service.read(cx);
        let account_uuid = account.as_ref().map(MicrosoftAccount::uuid);
        let profile_state = account_uuid.and_then(|uuid| service.profile_state(uuid));
        let profile = account_uuid.and_then(|uuid| service.profile(uuid));
        let texture_state = account_uuid.and_then(|uuid| service.texture_state(uuid));
        let profile_error = profile_state.and_then(|state| state.error());
        let texture_error = texture_state.and_then(|state| state.error());
        let profile_loading = profile_state.is_some_and(|state| state.is_loading());
        let can_apply = account.is_some();

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
                    .child(
                        card(cx)
                            .id("skin-library-card")
                            .debug_selector(|| "skin-library-card".to_owned())
                            .h_full()
                            .min_h_0()
                            .when_some(account.clone(), |this, account| {
                                this.child(self.render_profile_summary(
                                    account,
                                    profile,
                                    profile_loading,
                                    cx,
                                ))
                                .when_some(profile_error, |this, error| {
                                    this.child(error_banner(tr("app.skin.sync_failed"), error, cx))
                                })
                                .when_some(
                                    texture_error,
                                    |this, error| {
                                        this.child(error_banner(
                                            tr("app.skin.preview_unavailable"),
                                            error,
                                            cx,
                                        ))
                                    },
                                )
                            })
                            .when(!can_apply, |this| {
                                this.child(self.render_skin_account_notice(
                                    selected_is_authlib,
                                    selected_is_offline,
                                    cx,
                                ))
                            })
                            .child(self.render_skin_tabs(cx))
                            .child(
                                v_flex()
                                    .id("skin-library-body")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scrollbar()
                                    .child(match self.skin_tab {
                                        SkinTab::Online => {
                                            self.render_skin_catalog(cx).into_any_element()
                                        }
                                        SkinTab::Local => {
                                            self.render_skin_local(cx).into_any_element()
                                        }
                                    }),
                            )
                            .when(self.skin_editor.draft.is_some(), |this| {
                                this.child(self.render_skin_candidate(cx))
                            }),
                    ),
            )
    }

    fn render_skin_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        TabBar::new("skin-source-tabs")
            .segmented()
            .selected_index(self.skin_tab.index())
            .on_click(cx.listener(|this, index: &usize, _, cx| {
                this.skin_tab = SkinTab::from_index(*index);
                cx.notify();
            }))
            .children(
                SkinTab::ALL
                    .into_iter()
                    .map(|tab| Tab::new().label(tab.label())),
            )
    }

    fn render_skin_account_notice(
        &self,
        selected_is_authlib: bool,
        selected_is_offline: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let open_accounts = cx.entity().downgrade();
        let message = if selected_is_authlib {
            tr("app.skin.notice_authlib")
        } else if selected_is_offline {
            tr("app.skin.notice_offline")
        } else {
            tr("app.skin.notice_no_account")
        };
        v_flex()
            .gap_2()
            .flex_shrink_0()
            .child(warning_banner(tr("app.skin.cannot_change"), message, cx))
            .child(
                h_flex().child(
                    Button::new("skin-manage-accounts")
                        .label(tr("app.skin.manage_accounts"))
                        .outline()
                        .small()
                        .on_click(move |_, window, cx| {
                            let Some(view) = open_accounts.upgrade() else {
                                return;
                            };
                            view.update(cx, |view, cx| view.open_accounts_dialog(window, cx));
                        }),
                ),
            )
    }

    fn render_profile_summary(
        &self,
        account: MicrosoftAccount,
        profile: Option<&super::super::skin_service::LoadedProfile>,
        loading: bool,
        cx: &App,
    ) -> impl IntoElement {
        let profile = profile.map(|loaded| &loaded.profile).unwrap_or(&account);
        let account_uuid = profile.uuid();
        let head = self.account_heads.get(&account_uuid);
        let active_skin = profile.active_skin();
        let model = active_skin
            .map(|skin| variant_label(skin.variant()))
            .unwrap_or_else(|| tr("app.skin.default_model"));
        let skin_name = active_skin
            .and_then(|skin| skin.alias())
            .filter(|alias| !alias.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if active_skin.is_some() {
                    tr("app.skin.official_skin")
                } else {
                    tr("app.skin.default_skin")
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
                        layout::AVATAR_LARGE,
                        cx,
                    ))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_lg()
                                            .font_semibold()
                                            .truncate()
                                            .child(profile.username().to_owned()),
                                    )
                                    .when(loading, |this| {
                                        this.child(
                                            Spinner::new()
                                                .with_size(Size::XSmall)
                                                .color(cx.theme().primary),
                                        )
                                    }),
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
                        .child(rust_i18n::t!("app.skin.cape", cape = cape).to_string()),
                )
            })
    }

    fn render_skin_catalog(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (items, loading, error, selection_error, has_more, page) = {
            let catalog = self.skin_service.read(cx).catalog();
            (
                catalog.items().to_vec(),
                catalog.is_loading(),
                catalog.error().map(str::to_owned),
                catalog.selection_error().map(str::to_owned),
                catalog.has_more(),
                catalog.page(),
            )
        };
        let has_error = error.is_some();
        let sort = self.skin_catalog_sort;
        let search_view = cx.entity().downgrade();
        let load_view = cx.entity().downgrade();
        let retry_view = cx.entity().downgrade();
        let error_retry_view = cx.entity().downgrade();
        let more_view = cx.entity().downgrade();
        let more_empty_view = cx.entity().downgrade();
        let cards = items
            .iter()
            .map(|item| {
                let (preview, thumb_loading, thumb_failed, selected, selecting) = {
                    let catalog = self.skin_service.read(cx).catalog();
                    let (preview, thumb_loading, thumb_failed) = match catalog.thumbnail(&item.id) {
                        Some(CatalogThumbState::Ready { image, .. }) => {
                            (Some(image.clone()), false, false)
                        }
                        Some(CatalogThumbState::Loading) => (None, true, false),
                        Some(CatalogThumbState::Failed) => (None, false, true),
                        None => (None, true, false),
                    };
                    (
                        preview,
                        thumb_loading,
                        thumb_failed,
                        catalog.is_selected(&item.id),
                        catalog.is_selecting(&item.id),
                    )
                };
                self.render_skin_catalog_card(
                    item,
                    preview,
                    thumb_loading,
                    thumb_failed,
                    selected,
                    selecting,
                    cx,
                )
                .into_any_element()
            })
            .collect::<Vec<_>>();
        let sort_index = SkinCatalogSort::ALL
            .iter()
            .position(|candidate| *candidate == sort)
            .unwrap_or(0);

        v_flex()
            .gap_4()
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        Input::new(&self.skin_catalog_query)
                            .cleanable(true)
                            .w_full(),
                    )
                    .child(
                        Button::new("skin-catalog-search")
                            .label(tr("app.common.search"))
                            .icon(IconName::Search)
                            .primary()
                            .disabled(loading)
                            .on_click(move |_, window, cx| {
                                let Some(view) = search_view.upgrade() else {
                                    return;
                                };
                                view.update(cx, |view, cx| view.search_skin_catalog(window, cx));
                            }),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(
                        TabBar::new("skin-catalog-sort")
                            .segmented()
                            .selected_index(sort_index)
                            .on_click(cx.listener(|this, index: &usize, window, cx| {
                                this.skin_catalog_sort = SkinCatalogSort::ALL
                                    .get(*index)
                                    .copied()
                                    .unwrap_or_default();
                                this.reload_skin_catalog(window, cx);
                            }))
                            .children(
                                SkinCatalogSort::ALL.into_iter().map(|sort| {
                                    Tab::new().label(super::super::skin_sort_label(sort))
                                }),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if page == 0 {
                                "LittleSkin".to_owned()
                            } else {
                                rust_i18n::t!("app.skin.catalog_page", page = page).to_string()
                            }),
                    ),
            )
            .when_some(
                error.clone().filter(|_| !items.is_empty()),
                |this, error| {
                    this.child(error_banner(tr("app.skin.catalog_load_failed"), error, cx))
                },
            )
            .when_some(selection_error, |this, error| {
                this.child(warning_banner(tr("app.skin.cannot_preview"), error, cx))
            })
            .child(if loading && items.is_empty() {
                v_flex()
                    .w_full()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .py_12()
                    .child(
                        Spinner::new()
                            .with_size(Size::Medium)
                            .color(cx.theme().primary),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr("app.skin.catalog_loading")),
                    )
                    .into_any_element()
            } else if page == 0 && items.is_empty() && !has_error {
                empty_state(
                    IconName::Globe,
                    tr("app.skin.catalog_not_loaded"),
                    tr("app.skin.catalog_not_loaded_description"),
                    Button::new("skin-catalog-load")
                        .label(tr("app.skin.load_catalog"))
                        .outline()
                        .on_click(move |_, window, cx| {
                            let Some(view) = load_view.upgrade() else {
                                return;
                            };
                            view.update(cx, |view, cx| view.reload_skin_catalog(window, cx));
                        }),
                    cx,
                )
                .into_any_element()
            } else if items.is_empty() && has_error {
                let description = error
                    .as_deref()
                    .map(|error| {
                        rust_i18n::t!("app.skin.catalog_error_with_loaded", error = error)
                            .to_string()
                    })
                    .unwrap_or_else(|| tr("app.skin.catalog_error_retry"));
                empty_state(
                    IconName::Globe,
                    tr("app.skin.catalog_unavailable"),
                    description,
                    Button::new("skin-catalog-error-retry")
                        .debug_selector(|| "skin-catalog-error-retry".to_owned())
                        .label(if page == 0 {
                            tr("app.skin.reload")
                        } else {
                            tr("app.skin.retry_page")
                        })
                        .outline()
                        .on_click(move |_, window, cx| {
                            let Some(view) = error_retry_view.upgrade() else {
                                return;
                            };
                            if page == 0 {
                                view.update(cx, |view, cx| view.reload_skin_catalog(window, cx));
                            } else {
                                view.update(cx, |view, cx| {
                                    view.skin_service
                                        .update(cx, |service, cx| service.load_more_catalog(cx));
                                });
                            }
                        }),
                    cx,
                )
                .into_any_element()
            } else if items.is_empty() && has_more {
                empty_state(
                    IconName::Search,
                    tr("app.skin.page_empty"),
                    tr("app.skin.page_empty_description"),
                    Button::new("skin-catalog-more-empty")
                        .debug_selector(|| "skin-catalog-more-empty".to_owned())
                        .label(tr("app.skin.load_next_page"))
                        .outline()
                        .on_click(move |_, _, cx| {
                            let Some(view) = more_empty_view.upgrade() else {
                                return;
                            };
                            view.update(cx, |view, cx| {
                                view.skin_service
                                    .update(cx, |service, cx| service.load_more_catalog(cx));
                            });
                        }),
                    cx,
                )
                .into_any_element()
            } else if items.is_empty() {
                empty_state(
                    IconName::Search,
                    tr("app.skin.no_results"),
                    tr("app.skin.no_results_description"),
                    Button::new("skin-catalog-retry")
                        .label(tr("app.skin.reload"))
                        .outline()
                        .on_click(move |_, window, cx| {
                            let Some(view) = retry_view.upgrade() else {
                                return;
                            };
                            view.update(cx, |view, cx| view.reload_skin_catalog(window, cx));
                        }),
                    cx,
                )
                .into_any_element()
            } else {
                div()
                    .w_full()
                    .grid()
                    .grid_cols(2)
                    .gap_3()
                    .children(cards)
                    .into_any_element()
            })
            .when(has_more && !items.is_empty(), |this| {
                this.child(
                    Button::new("skin-catalog-more")
                        .label(if loading {
                            tr("app.content_browser.loading_more")
                        } else {
                            tr("app.content_browser.load_more")
                        })
                        .outline()
                        .w_full()
                        .disabled(loading)
                        .on_click(move |_, _, cx| {
                            let Some(view) = more_view.upgrade() else {
                                return;
                            };
                            view.update(cx, |view, cx| {
                                view.skin_service
                                    .update(cx, |service, cx| service.load_more_catalog(cx));
                            });
                        }),
                )
            })
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr("app.skin.standard_only")),
                    )
                    .child(skin_site_button(
                        "littleskin",
                        tr("app.skin.open_littleskin"),
                        "https://littleskin.cn/skinlib",
                    )),
            )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_skin_catalog_card(
        &self,
        item: &SkinCatalogItem,
        preview: Option<Arc<RenderImage>>,
        thumb_loading: bool,
        thumb_failed: bool,
        selected: bool,
        selecting: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let border = if selected {
            theme.primary
        } else {
            theme.border
        };
        let background = if selected {
            theme.primary.opacity(0.06)
        } else {
            theme.group_box
        };
        let item_for_click = item.clone();
        let author = item
            .author
            .clone()
            .unwrap_or_else(|| "LittleSkin".to_owned());
        let author_line = format!("{author} · {}", variant_label(item.variant));
        let debug_selector = format!("skin-catalog-card-{}", item.id);
        Button::new(format!("skin-catalog-card-{}", item.id))
            .ghost()
            .debug_selector(move || debug_selector.clone())
            .w_full()
            .h_auto()
            .p_0()
            .border_1()
            .border_color(border)
            .bg(background)
            .cursor_pointer()
            .accessibility_label(
                rust_i18n::t!("app.skin.preview_skin", name = item.name).to_string(),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_catalog_skin(item_for_click.clone(), cx);
            }))
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_0()
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .h(px(176.))
                            .rounded(theme.radius)
                            .bg(theme.muted)
                            .overflow_hidden()
                            .when_some(preview, |this, image| {
                                this.child(
                                    img(image)
                                        .size_full()
                                        .object_fit(ObjectFit::Contain)
                                        .rounded(theme.radius),
                                )
                            })
                            .when(thumb_loading, |this| {
                                this.child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .flex()
                                        .flex_col()
                                        .items_center()
                                        .justify_center()
                                        .gap_1()
                                        .child(
                                            Icon::new(IconName::Shirt)
                                                .small()
                                                .text_color(theme.muted_foreground.opacity(0.6)),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(theme.muted_foreground)
                                                .child(tr("app.skin.rendering")),
                                        ),
                                )
                            })
                            .when(thumb_failed, |this| {
                                this.child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            Icon::new(IconName::Shirt)
                                                .small()
                                                .text_color(theme.muted_foreground),
                                        ),
                                )
                            })
                            .when(selected, |this| {
                                this.child(
                                    div()
                                        .absolute()
                                        .top_2()
                                        .right_2()
                                        .size_5()
                                        .rounded_full()
                                        .bg(theme.primary)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .when(selecting, |this| {
                                            this.child(
                                                Spinner::new()
                                                    .with_size(Size::XSmall)
                                                    .color(theme.primary_foreground),
                                            )
                                        })
                                        .when(!selecting, |this| {
                                            this.child(
                                                Icon::new(IconName::Check)
                                                    .small()
                                                    .text_color(theme.primary_foreground),
                                            )
                                        }),
                                )
                            }),
                    )
                    .child(
                        v_flex()
                            .w_full()
                            .min_w_0()
                            .gap_1()
                            .p_2()
                            .child(
                                div()
                                    .w_full()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .font_medium()
                                    .child(item.name.clone()),
                            )
                            .child(
                                h_flex()
                                    .w_full()
                                    .min_w_0()
                                    .justify_between()
                                    .gap_2()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .child(author_line),
                                    )
                                    .child(
                                        h_flex()
                                            .flex_shrink_0()
                                            .items_center()
                                            .gap_1()
                                            .child(
                                                Icon::new(IconName::Heart)
                                                    .with_size(Size::XSmall)
                                                    .text_color(theme.muted_foreground),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(theme.muted_foreground)
                                                    .child(item.likes.to_string()),
                                            ),
                                    ),
                            ),
                    ),
            )
    }

    fn render_skin_local(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_5()
            .child(section_title(
                tr("app.skin.upload_or_link"),
                tr("app.skin.upload_or_link_description"),
                cx,
            ))
            .child(self.render_skin_import(cx))
            .child(div().h(px(1.)).bg(cx.theme().border))
            .child(self.render_recent_skins(cx))
    }

    fn render_skin_candidate(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(draft) = self.skin_editor.draft.clone() else {
            return div().into_any_element();
        };
        let applying = self.skin_apply.is_applying();
        let cooldown = self.skin_apply.cooldown_remaining();
        let can_apply = self.selected_microsoft_account().is_some();
        let catalog_item = self.skin_editor.catalog_item.clone();
        let title = catalog_item
            .as_ref()
            .map(|item| item.name.clone())
            .unwrap_or_else(|| draft.file_name().to_owned());
        let source = catalog_item
            .as_ref()
            .map(|item| {
                item.author
                    .as_ref()
                    .map(|author| format!("LittleSkin · {author}"))
                    .unwrap_or_else(|| "LittleSkin".to_owned())
            })
            .or_else(|| {
                draft
                    .source_url()
                    .map(|url| rust_i18n::t!("app.skin.source", url = url).to_string())
            })
            .unwrap_or_else(|| tr("app.skin.local_or_link_import"));
        let site_url = catalog_item.as_ref().map(|item| item.site_url.clone());

        v_flex()
            .flex_shrink_0()
            .gap_3()
            .pt_4()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .w_full()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .size_8()
                            .flex_shrink_0()
                            .rounded(cx.theme().radius)
                            .bg(cx.theme().primary.opacity(0.12))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                Icon::new(IconName::Shirt)
                                    .small()
                                    .text_color(cx.theme().primary),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .font_medium()
                                    .child(title),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(source),
                            ),
                    )
                    .when_some(site_url, |this, url| {
                        this.child(
                            Button::new("skin-candidate-site")
                                .icon(IconName::ExternalLink)
                                .ghost()
                                .small()
                                .tooltip(tr("app.skin.open_skin_page"))
                                .on_click(move |_, _, cx| cx.open_url(&url)),
                        )
                    }),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(field_label(tr("app.skin.model")))
                    .child(
                        Button::new("skin-model-classic")
                            .label(tr("app.skin.classic"))
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
                            .label(tr("app.skin.slim"))
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
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("skin-apply")
                            .label(if applying {
                                tr("app.skin.applying")
                            } else if cooldown.is_some() {
                                tr("app.skin.cooldown")
                            } else if can_apply {
                                tr("app.skin.confirm_apply")
                            } else {
                                tr("app.skin.requires_microsoft")
                            })
                            .primary()
                            .disabled(!self.skin_apply.is_idle() || !can_apply)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.apply_skin(window, cx);
                            })),
                    )
                    .child(
                        Button::new("skin-cancel-draft")
                            .label(tr("app.common.cancel"))
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
                                .child(
                                    rust_i18n::t!(
                                        "app.skin.cooldown_remaining",
                                        seconds = remaining.as_secs().max(1)
                                    )
                                    .to_string(),
                                ),
                        )
                    })
                    .when(!can_apply, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr("app.skin.login_to_change")),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_skin_import(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                            .label(tr("app.skin.upload_png"))
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
                            .child(tr("app.skin.png_requirements")),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(field_label(tr("app.skin.image_url")))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Input::new(&self.skin_link.input).w_full())
                            .child(
                                Button::new("skin-read-link")
                                    .label(if link_loading {
                                        tr("app.skin.reading")
                                    } else {
                                        tr("app.skin.read")
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
                        this.child(warning_banner(tr("app.skin.link_read_failed"), error, cx))
                    })
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr("app.skin.direct_png_hint")),
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
                        tr("app.skin.recent"),
                        tr("app.skin.recent_description"),
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
                                .label(tr("app.common.retry"))
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
                this.child(warning_banner(tr("app.skin.recent_load_failed"), error, cx))
            })
            .when(entries.is_empty() && !loading, |this| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("app.skin.recent_empty")),
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
                    .label(tr("app.skin.use"))
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
        let account = self.selected_microsoft_account().cloned();
        self.skin_service.update(cx, |service, cx| {
            service.ensure_catalog(cx);
            service.ensure_recent(cx);
            if let Some(account) = account {
                service.ensure_profile(account, cx);
            }
        });
    }

    fn flush_pending_catalog_skin(&mut self, cx: &mut Context<Self>) {
        let Some(loaded) = self.pending_catalog_skin.take() else {
            return;
        };
        self.skin_editor.variant = loaded.item.variant;
        self.skin_editor.catalog_item = Some(loaded.item);
        self.skin_editor.draft = Some(loaded.skin);
        self.skin_viewer
            .update(cx, |viewer, cx| viewer.reset_view(cx));
        cx.notify();
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

    fn search_skin_catalog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.skin_catalog_query.read(cx).value().trim().to_owned();
        self.reload_skin_catalog_with(window, query, self.skin_catalog_sort, cx);
    }

    fn reload_skin_catalog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.skin_catalog_query.read(cx).value().trim().to_owned();
        self.reload_skin_catalog_with(window, query, self.skin_catalog_sort, cx);
    }

    fn reload_skin_catalog_with(
        &mut self,
        window: &mut Window,
        query: String,
        sort: SkinCatalogSort,
        cx: &mut Context<Self>,
    ) {
        let thumbnails = self
            .skin_service
            .update(cx, |service, _| service.take_catalog_thumbnails());
        for image in thumbnails {
            let _ = window.drop_image(image);
        }
        self.skin_service
            .update(cx, |service, cx| service.reload_catalog(query, sort, cx));
    }

    fn select_catalog_skin(&mut self, item: SkinCatalogItem, cx: &mut Context<Self>) {
        self.skin_service
            .update(cx, |service, cx| service.select_catalog_item(item, cx));
    }

    fn pick_skin_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("app.skin.choose_png").into()),
        });
        self._prompt_task = Some(
            cx.spawn_in(window, async move |this, cx| match prompt.await {
                Ok(Ok(Some(mut paths))) if !paths.is_empty() => {
                    let path = paths.remove(0);
                    let _ = this
                        .update_in(cx, |view, window, cx| view.load_skin_path(path, window, cx));
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    let _ = this.update_in(cx, |_, window, cx| {
                        window.push_notification(
                            Notification::error(
                                rust_i18n::t!("app.common.file_picker_failed", error = error)
                                    .to_string(),
                            )
                            .title(tr("app.skin.choose_failed")),
                            cx,
                        );
                    });
                }
                Err(_) => {}
            }),
        );
        cx.notify();
    }

    fn load_skin_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("png"))
        {
            window.push_notification(
                Notification::warning(tr("app.skin.choose_png_warning"))
                    .title(tr("app.skin.import_failed")),
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
                            .title(tr("app.skin.import_failed")),
                        cx,
                    );
                }
                Err(error) => {
                    window.push_notification(
                        Notification::error(
                            rust_i18n::t!("app.skin.process_task_failed", error = error)
                                .to_string(),
                        )
                        .title(tr("app.skin.import_failed")),
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
            self.skin_link.state = LinkImportState::Failed(tr("app.skin.enter_image_url"));
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
                        view.skin_link.state = LinkImportState::Failed(
                            rust_i18n::t!("app.skin.read_task_failed", error = error).to_string(),
                        );
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
        self.skin_editor.catalog_item = None;
        self.skin_service
            .update(cx, |service, cx| service.clear_catalog_selection(cx));
        self.skin_viewer
            .update(cx, |viewer, cx| viewer.reset_view(cx));
        cx.notify();
    }

    fn cancel_skin_draft(&mut self, cx: &mut Context<Self>) {
        self.skin_editor.draft = None;
        self.skin_editor.catalog_item = None;
        self.skin_service
            .update(cx, |service, cx| service.clear_catalog_selection(cx));
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
                        Notification::error(
                            rust_i18n::t!("app.skin.read_local_failed", error = error).to_string(),
                        )
                        .title(tr("app.skin.use_failed")),
                        cx,
                    );
                }
                Err(error) => {
                    window.push_notification(
                        Notification::error(
                            rust_i18n::t!("app.skin.local_task_failed", error = error).to_string(),
                        )
                        .title(tr("app.skin.use_failed")),
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
                Notification::warning(tr("app.skin.select_microsoft_notice"))
                    .title(tr("app.skin.apply_failed")),
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
                        view.skin_editor.catalog_item = None;
                        view.skin_editor.variant = variant;
                        view.skin_service
                            .update(cx, |service, cx| service.clear_catalog_selection(cx));
                        view.start_skin_cooldown(Duration::from_secs(2), cx);
                        window.push_notification(
                            Notification::success(tr("app.skin.applied_notice"))
                                .title(tr("app.skin.applied")),
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
                            Notification::error(message).title(tr("app.skin.change_failed")),
                            cx,
                        );
                    }
                    Err(error) => {
                        view.skin_apply = ApplyState::Idle;
                        window.push_notification(
                            Notification::error(
                                rust_i18n::t!("app.skin.upload_task_failed", error = error)
                                    .to_string(),
                            )
                            .title(tr("app.skin.change_failed")),
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

fn variant_label(variant: SkinVariant) -> String {
    tr(match variant {
        SkinVariant::Classic => "app.skin.classic_model",
        SkinVariant::Slim => "app.skin.slim_model",
    })
}

fn skin_site_button(id: &'static str, label: impl Into<SharedString>, url: &'static str) -> Button {
    Button::new(format!("skin-site-{id}"))
        .label(label)
        .link()
        .small()
        .icon(IconName::ExternalLink)
        .tooltip(tr("app.skin.open_in_browser"))
        .on_click(move |_, _, cx| cx.open_url(url))
}

fn describe_apply_error(error: &MicrosoftAccountServiceError) -> (String, Option<Duration>) {
    match error {
        MicrosoftAccountServiceError::CredentialsMissing
        | MicrosoftAccountServiceError::Auth(
            MicrosoftAuthError::ReauthenticationRequired
            | MicrosoftAuthError::MinecraftTokenRejected,
        ) => (tr("app.skin.error_login_expired"), None),
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::SkinRateLimited { retry_after }) => {
            (
                tr("app.skin.error_rate_limited"),
                Some(Duration::from_secs(retry_after.unwrap_or(30))),
            )
        }
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::SkinRejected { message }) => (
            rust_i18n::t!("app.skin.error_rejected", message = message).to_string(),
            None,
        ),
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::SkinForbidden { .. }) => {
            (tr("app.skin.error_forbidden"), None)
        }
        MicrosoftAccountServiceError::Auth(
            MicrosoftAuthError::Http { .. } | MicrosoftAuthError::HttpStatus { .. },
        ) => (tr("app.skin.error_network"), None),
        other => (other.to_string(), None),
    }
}

fn describe_skin_source_error(error: &SkinSourceError) -> String {
    match error {
        SkinSourceError::UnsupportedScheme | SkinSourceError::CredentialsNotAllowed => {
            tr("app.skin.error_https")
        }
        SkinSourceError::PrivateAddress => tr("app.skin.error_private_address"),
        SkinSourceError::NotPng => tr("app.skin.error_not_png"),
        SkinSourceError::HttpStatus { status } => {
            rust_i18n::t!("app.skin.error_http_status", status = status).to_string()
        }
        SkinSourceError::SourceTooLarge => tr("app.skin.error_source_too_large"),
        SkinSourceError::UnsupportedUploadDimensions { width, height } => {
            rust_i18n::t!("app.skin.error_dimensions", width = width, height = height).to_string()
        }
        SkinSourceError::UploadTooLarge => tr("app.skin.error_upload_too_large"),
        SkinSourceError::Skin(_) => tr("app.skin.error_invalid_png"),
        SkinSourceError::InvalidUrl(_) | SkinSourceError::MissingHost => {
            tr("app.skin.error_invalid_url")
        }
        SkinSourceError::Http(_) | SkinSourceError::Resolve(_) => tr("app.skin.error_connect"),
        SkinSourceError::Cancelled => tr("app.skin.error_cancelled"),
        SkinSourceError::TooManyRedirects | SkinSourceError::MissingLocation => {
            tr("app.skin.error_redirects")
        }
        SkinSourceError::Encode(_) => tr("app.skin.error_encode"),
        SkinSourceError::Io { .. } => tr("app.skin.error_read_file"),
    }
}
