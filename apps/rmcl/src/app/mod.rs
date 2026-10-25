mod ui;

use std::{cell::Cell, collections::HashMap, path::PathBuf, rc::Rc, sync::Arc, time::Duration};

use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Icon, IndexPath, Sizable, Size, StyledExt, VirtualListScrollHandle, WindowExt,
        button::{Button, ButtonVariant, ButtonVariants},
        dialog::{DialogAction, DialogClose, DialogFooter},
        h_flex,
        input::{Input, InputEvent, InputState},
        notification::Notification,
        scroll::ScrollableElement,
        searchable_list::SearchableListItem,
        select::{SearchableVec, SelectEvent, SelectState},
        spinner::Spinner,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use rmcl_app::{
    AppError, GameLaunchRequest, InstallPhase, InstalledVersion, MicrosoftAccountServiceError,
    ResolvedMicrosoftAccount, TaskEvent, TaskKind, TaskPhase, TaskProgress, TaskSnapshot,
    TaskStatus, VanillaInstallRequest, VersionManifest, VersionSource, VersionSummary,
    delete_installed_version, discover_installed_versions,
};
use rmcl_auth::{Account, AccountKind, MicrosoftAccount, MicrosoftAuthError, OfflineAccount};
use rmcl_domain::{Architecture, OperatingSystem, Platform};
use rmcl_launch::{GameLog, LaunchAccount, LogStream};
use rmcl_store::{
    AccentColor, CONFIG_SCHEMA_VERSION, LauncherConfig, LauncherVisibility, StoredAccountKind,
    ThemePreference,
};
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    services::{
        Bootstrap, Services, load_account, load_microsoft_account, open_path, stored_account,
        stored_microsoft_account,
    },
    state::LauncherState,
    theme,
};

const THEME_ITEMS: [&str; 3] = ["跟随系统", "浅色", "深色"];
const ACCENT_ITEMS: [&str; 5] = ["翡翠", "海蓝", "紫罗兰", "琥珀", "玫瑰"];
const VISIBILITY_ITEMS: [&str; 2] = ["保持显示", "游戏运行时隐藏"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Home,
    Versions,
    Downloads,
    Logs,
    Settings,
}

impl Page {
    pub fn title(self) -> &'static str {
        match self {
            Self::Home => "主页",
            Self::Versions => "版本",
            Self::Downloads => "下载游戏",
            Self::Logs => "日志",
            Self::Settings => "设置",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Self::Home => "选择一个版本，立即开始游戏。",
            Self::Versions => "管理已安装的 Minecraft 版本。",
            Self::Downloads => "从 Mojang 官方源安装新的 Vanilla 版本。",
            Self::Logs => "查看安装、启动和游戏运行日志。",
            Self::Settings => "配置游戏目录、外观与 Java。",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Home => IconName::House,
            Self::Versions => IconName::Boxes,
            Self::Downloads => IconName::Download,
            Self::Logs => IconName::ScrollText,
            Self::Settings => IconName::Settings,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VersionFilter {
    #[default]
    All,
    Release,
    Snapshot,
    Old,
}

impl VersionFilter {
    pub const ALL: [Self; 4] = [Self::All, Self::Release, Self::Snapshot, Self::Old];
    pub const DOWNLOADS: [Self; 3] = [Self::Release, Self::Snapshot, Self::Old];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Release => "正式版",
            Self::Snapshot => "快照",
            Self::Old => "旧版",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::All => 0,
            Self::Release => 1,
            Self::Snapshot => 2,
            Self::Old => 3,
        }
    }

    fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(Self::All)
    }

    fn download_index(self) -> usize {
        match self {
            Self::Release => 0,
            Self::Snapshot => 1,
            Self::Old => 2,
            Self::All => 0,
        }
    }

    fn from_download_index(index: usize) -> Self {
        Self::DOWNLOADS.get(index).copied().unwrap_or(Self::Release)
    }

    fn matches(self, version_type: Option<&str>) -> bool {
        match self {
            Self::All => true,
            Self::Release => version_type == Some("release"),
            Self::Snapshot => version_type == Some("snapshot"),
            Self::Old => version_type.is_some_and(|kind| kind.starts_with("old_")),
        }
    }
}

#[derive(Default)]
pub struct CatalogState {
    manifest: Option<Arc<VersionManifest>>,
    loading: bool,
    error: Option<String>,
}

enum Command {
    Install(VanillaInstallRequest),
    Launch(GameLaunchRequest),
}

#[derive(Clone)]
enum MicrosoftLoginUi {
    Requesting,
    Waiting {
        user_code: String,
        verification_uri: String,
    },
    Completing,
    Failed {
        message: String,
    },
}

#[derive(Clone, Debug)]
struct AccountSelectItem {
    key: String,
    username: String,
}

impl AccountSelectItem {
    fn new(account: &Account) -> Self {
        Self {
            key: account_key(account),
            username: account.username().to_owned(),
        }
    }
}

impl SearchableListItem for AccountSelectItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.username.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.key
    }
}

pub struct LauncherApp {
    services: Rc<Services>,
    page: Page,
    config: LauncherConfig,
    config_warning: Option<String>,
    config_save_error: Option<String>,
    installed_versions: Vec<InstalledVersion>,
    versions_error: Option<String>,
    accounts: Vec<Account>,
    account_heads: HashMap<Uuid, PathBuf>,
    editing_account: Option<String>,
    version_select: Entity<SelectState<SearchableVec<String>>>,
    account_select: Entity<SelectState<SearchableVec<AccountSelectItem>>>,
    theme_select: Entity<SelectState<SearchableVec<String>>>,
    accent_select: Entity<SelectState<SearchableVec<String>>>,
    visibility_select: Entity<SelectState<SearchableVec<String>>>,
    log_state: LauncherState,
    log_scroll: ScrollHandle,
    log_follow: bool,
    catalog: CatalogState,
    downloads_query: Entity<InputState>,
    versions_query: Entity<InputState>,
    account_input: Entity<InputState>,
    downloads_filter: VersionFilter,
    versions_filter: VersionFilter,
    downloads_scroll: VirtualListScrollHandle,
    active_task_version: Option<String>,
    launcher_hidden: bool,
    logs_attention: bool,
    launch_resolving: bool,
    microsoft_login: Option<MicrosoftLoginUi>,
    login_cancel: Option<CancellationToken>,
    login_code_copied: bool,
    _event_task: Task<()>,
    _command_task: Option<Task<()>>,
    _prompt_task: Option<Task<()>>,
    _catalog_task: Option<Task<()>>,
    _launch_task: Option<Task<()>>,
    _login_task: Option<Task<()>>,
    _copy_feedback_task: Option<Task<()>>,
    _skin_task: Option<Task<()>>,
    _profile_task: Option<Task<()>>,
    _account_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
    _appearance_subscription: Subscription,
}

impl LauncherApp {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        services: Rc<Services>,
        bootstrap: Bootstrap,
    ) -> Self {
        let save_on_start = bootstrap.save_on_start;
        let mut config = bootstrap.config;
        let config_warning = bootstrap.config_warning;
        let mut accounts = Vec::new();
        accounts.extend(
            config
                .accounts
                .iter()
                .filter_map(load_account)
                .map(Account::Offline),
        );
        accounts.extend(
            config
                .microsoft_accounts
                .iter()
                .filter_map(load_microsoft_account)
                .map(Account::Microsoft),
        );
        if !accounts.iter().any(|account| {
            config.selected_account.as_deref() == Some(account.username())
                && stored_account_kind(account) == config.selected_account_kind
        }) {
            config.selected_account = accounts
                .first()
                .map(|account| account.username().to_owned());
            config.selected_account_kind = accounts
                .first()
                .map(stored_account_kind)
                .unwrap_or(StoredAccountKind::Offline);
        }

        let mut versions_error = None;
        let installed_versions = match discover_installed_versions(&config.minecraft_root) {
            Ok(versions) => versions,
            Err(error) => {
                versions_error = Some(error.to_string());
                Vec::new()
            }
        };
        if !installed_versions
            .iter()
            .any(|version| Some(version.id.as_str()) == config.selected_version.as_deref())
        {
            config.selected_version = installed_versions
                .first()
                .map(|version| version.id.as_str().to_owned());
        }
        if save_on_start {
            let _ = services.config_store.save(&config);
        }

        let version_items = SearchableVec::new(
            installed_versions
                .iter()
                .map(|version| version.id.as_str().to_owned())
                .collect::<Vec<_>>(),
        );
        let version_index = config.selected_version.as_ref().and_then(|selected| {
            installed_versions
                .iter()
                .position(|version| version.id.as_str() == selected)
                .map(|row| IndexPath::default().row(row))
        });
        let version_select = cx
            .new(|cx| SelectState::new(version_items, version_index, window, cx).searchable(true));

        let account_items = SearchableVec::new(
            accounts
                .iter()
                .map(AccountSelectItem::new)
                .collect::<Vec<_>>(),
        );
        let account_index = config.selected_account.as_ref().and_then(|selected| {
            accounts
                .iter()
                .position(|account| {
                    account.username() == selected
                        && stored_account_kind(account) == config.selected_account_kind
                })
                .map(|row| IndexPath::default().row(row))
        });
        let account_select = cx
            .new(|cx| SelectState::new(account_items, account_index, window, cx).searchable(true));

        let theme_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(THEME_ITEMS.map(str::to_owned).to_vec()),
                Some(IndexPath::default().row(theme_index(config.appearance.theme))),
                window,
                cx,
            )
        });
        let accent_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(ACCENT_ITEMS.map(str::to_owned).to_vec()),
                Some(IndexPath::default().row(accent_index(config.appearance.accent))),
                window,
                cx,
            )
        });
        let visibility_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(VISIBILITY_ITEMS.map(str::to_owned).to_vec()),
                Some(IndexPath::default().row(visibility_index(config.launcher_visibility))),
                window,
                cx,
            )
        });

        let downloads_query =
            cx.new(|cx| InputState::new(window, cx).placeholder("搜索版本 ID，例如 1.21.1"));
        let versions_query = cx.new(|cx| InputState::new(window, cx).placeholder("搜索已安装版本"));
        let account_input = cx.new(|cx| InputState::new(window, cx).placeholder("离线玩家名"));

        let mut subscriptions = Vec::new();
        subscriptions.push(
            cx.subscribe(&versions_query, |_this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        );
        subscriptions.push(
            cx.subscribe(&downloads_query, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.downloads_scroll.scroll_to_item(0, ScrollStrategy::Top);
                    cx.notify();
                }
            }),
        );
        subscriptions.push(cx.subscribe(
            &version_select,
            |this, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    this.select_version(value.clone(), cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe(
            &account_select,
            |this, _, event: &SelectEvent<SearchableVec<AccountSelectItem>>, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    this.select_account(value.clone(), cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe(
            &theme_select,
            |this, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    this.set_theme(value.clone(), cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe(
            &accent_select,
            |this, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    this.set_accent(value.clone(), cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe(
            &visibility_select,
            |this, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    this.set_visibility(value.clone(), cx);
                }
            },
        ));

        let event_task = Self::spawn_event_bridge(window, &services, cx);
        let appearance_view = cx.entity().downgrade();
        let appearance_subscription = window.observe_window_appearance(move |_window, cx| {
            let _ = appearance_view.update_in(cx, |this, window, cx| {
                if this.config.appearance.theme == ThemePreference::System {
                    theme::apply(this.config.appearance, Some(window), cx);
                    cx.notify();
                }
            });
        });

        let mut app = Self {
            services,
            page: Page::Home,
            config,
            config_warning,
            config_save_error: None,
            installed_versions,
            versions_error,
            accounts,
            account_heads: HashMap::new(),
            editing_account: None,
            version_select,
            account_select,
            theme_select,
            accent_select,
            visibility_select,
            log_state: LauncherState::new(),
            log_scroll: ScrollHandle::new(),
            log_follow: true,
            catalog: CatalogState::default(),
            downloads_query,
            versions_query,
            account_input,
            downloads_filter: VersionFilter::Release,
            versions_filter: VersionFilter::All,
            downloads_scroll: VirtualListScrollHandle::new(),
            active_task_version: None,
            launcher_hidden: false,
            logs_attention: false,
            launch_resolving: false,
            microsoft_login: None,
            login_cancel: None,
            login_code_copied: false,
            _event_task: event_task,
            _command_task: None,
            _prompt_task: None,
            _catalog_task: None,
            _launch_task: None,
            _login_task: None,
            _copy_feedback_task: None,
            _skin_task: None,
            _profile_task: None,
            _account_task: None,
            _subscriptions: subscriptions,
            _appearance_subscription: appearance_subscription,
        };
        app.refresh_account_heads(window, cx);
        app.refresh_microsoft_profiles(window, cx);
        app
    }

    fn spawn_event_bridge(
        window: &mut Window,
        services: &Rc<Services>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let manager = services.tasks.clone();
        let mut events = manager.subscribe();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                match events.recv().await {
                    Ok(TaskEvent::Snapshot(snapshot)) => {
                        let _ = this.update_in(cx, |view, window, cx| {
                            let was_terminal =
                                view.log_state.snapshot.as_ref().is_some_and(|current| {
                                    current.id == snapshot.id && current.status.is_terminal()
                                });
                            let should_hide = snapshot.kind == TaskKind::GameLaunch
                                && snapshot.status == TaskStatus::Running
                                && snapshot.phase == Some(TaskPhase::RunningGame)
                                && view.config.launcher_visibility
                                    == LauncherVisibility::HideWhileRunning;
                            if view.log_state.apply_snapshot(snapshot.clone()) {
                                if should_hide {
                                    view.hide_for_game(window, cx);
                                }
                                if !was_terminal && snapshot.status.is_terminal() {
                                    view.on_task_terminal(snapshot, window, cx);
                                }
                                cx.notify();
                            }
                        });
                    }
                    Ok(TaskEvent::GameLog { task_id, line, log }) => {
                        let _ = this.update_in(cx, |view, _, cx| {
                            let was_at_bottom = view.log_scroll.offset().y
                                + view.log_scroll.max_offset().y
                                <= px(1.);
                            if view.log_state.apply_log(task_id, line, log) {
                                if view.log_follow && was_at_bottom {
                                    view.log_scroll.scroll_to_bottom();
                                } else if view.log_follow {
                                    view.log_follow = false;
                                }
                                cx.notify();
                            }
                        });
                    }
                    Ok(TaskEvent::Dismissed(_)) => {}
                    Err(RecvError::Lagged(_)) => {
                        let Some(id) = this
                            .update_in(cx, |view, _, _| view.log_state.active_id())
                            .ok()
                            .flatten()
                        else {
                            continue;
                        };
                        let snapshot = manager.snapshot(id).await;
                        let logs = manager.logs(id).await;
                        if let Some(snapshot) = snapshot
                            && this
                                .update_in(cx, |view, _, cx| {
                                    view.log_state.recover(snapshot, logs.unwrap_or_default());
                                    if view.log_follow {
                                        view.log_scroll.scroll_to_bottom();
                                    }
                                    cx.notify();
                                })
                                .is_err()
                        {
                            break;
                        }
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        })
    }

    fn select_version(&mut self, value: String, cx: &mut Context<Self>) {
        if self.config.selected_version.as_deref() == Some(value.as_str()) {
            return;
        }
        self.config.selected_version = Some(value);
        self.save_config();
        cx.notify();
    }

    fn select_account(&mut self, value: String, cx: &mut Context<Self>) {
        let Some(account) = self
            .accounts
            .iter()
            .find(|account| account_key(account) == value)
            .cloned()
        else {
            return;
        };
        let kind = stored_account_kind(&account);
        if self.config.selected_account.as_deref() == Some(account.username())
            && self.config.selected_account_kind == kind
        {
            return;
        }
        self.config.selected_account = Some(account.username().to_owned());
        self.config.selected_account_kind = kind;
        self.save_config();
        cx.notify();
    }

    fn select_account_key(
        &mut self,
        username: &str,
        kind: StoredAccountKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.config.selected_account.as_deref() == Some(username)
            && self.config.selected_account_kind == kind
        {
            return;
        }
        if !self
            .accounts
            .iter()
            .any(|account| account.username() == username && stored_account_kind(account) == kind)
        {
            return;
        }
        self.config.selected_account = Some(username.to_owned());
        self.config.selected_account_kind = kind;
        self.save_config();
        self.sync_account_select(window, cx);
        cx.notify();
    }

    fn set_theme(&mut self, value: String, cx: &mut Context<Self>) {
        let theme = match value.as_str() {
            "跟随系统" => ThemePreference::System,
            "浅色" => ThemePreference::Light,
            "深色" => ThemePreference::Dark,
            _ => return,
        };
        if self.config.appearance.theme == theme {
            return;
        }
        self.config.appearance.theme = theme;
        theme::apply(self.config.appearance, None, cx);
        self.save_config();
        cx.notify();
    }

    fn set_accent(&mut self, value: String, cx: &mut Context<Self>) {
        let accent = match value.as_str() {
            "翡翠" => AccentColor::Emerald,
            "海蓝" => AccentColor::Ocean,
            "紫罗兰" => AccentColor::Violet,
            "琥珀" => AccentColor::Amber,
            "玫瑰" => AccentColor::Rose,
            _ => return,
        };
        if self.config.appearance.accent == accent {
            return;
        }
        self.config.appearance.accent = accent;
        theme::apply(self.config.appearance, None, cx);
        self.save_config();
        cx.notify();
    }

    fn set_visibility(&mut self, value: String, cx: &mut Context<Self>) {
        let visibility = match value.as_str() {
            "保持显示" => LauncherVisibility::KeepVisible,
            "游戏运行时隐藏" => LauncherVisibility::HideWhileRunning,
            _ => return,
        };
        if self.config.launcher_visibility == visibility {
            return;
        }
        self.config.launcher_visibility = visibility;
        self.save_config();
        cx.notify();
    }

    fn toggle_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let theme = if cx.theme().mode.is_dark() {
            ThemePreference::Light
        } else {
            ThemePreference::Dark
        };
        self.config.appearance.theme = theme;
        theme::apply(self.config.appearance, Some(window), cx);
        self.theme_select.update(cx, |state, cx| {
            state.set_selected_index(
                Some(IndexPath::default().row(theme_index(theme))),
                window,
                cx,
            )
        });
        self.save_config();
        cx.notify();
    }

    fn save_config(&mut self) {
        self.config.schema_version = CONFIG_SCHEMA_VERSION;
        self.config.accounts = self
            .accounts
            .iter()
            .filter_map(|account| match account {
                Account::Offline(account) => Some(stored_account(account)),
                Account::Microsoft(_) => None,
            })
            .collect();
        self.config.microsoft_accounts = self
            .accounts
            .iter()
            .filter_map(|account| match account {
                Account::Microsoft(account) => Some(stored_microsoft_account(account)),
                Account::Offline(_) => None,
            })
            .collect();
        let selected_exists = self.accounts.iter().any(|account| {
            self.config.selected_account.as_deref() == Some(account.username())
                && stored_account_kind(account) == self.config.selected_account_kind
        });
        if !selected_exists {
            self.config.selected_account = self
                .accounts
                .first()
                .map(|account| account.username().to_owned());
            self.config.selected_account_kind = self
                .accounts
                .first()
                .map(stored_account_kind)
                .unwrap_or(StoredAccountKind::Offline);
        }
        match self.services.config_store.save(&self.config) {
            Ok(()) => self.config_save_error = None,
            Err(error) => self.config_save_error = Some(error.to_string()),
        }
    }

    fn reload_versions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match discover_installed_versions(&self.config.minecraft_root) {
            Ok(versions) => {
                self.installed_versions = versions;
                self.versions_error = None;
                if !self.installed_versions.iter().any(|version| {
                    Some(version.id.as_str()) == self.config.selected_version.as_deref()
                }) {
                    self.config.selected_version = self
                        .installed_versions
                        .first()
                        .map(|version| version.id.as_str().to_owned());
                }
                self.sync_version_select(window, cx);
            }
            Err(error) => self.versions_error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn sync_version_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = SearchableVec::new(
            self.installed_versions
                .iter()
                .map(|version| version.id.as_str().to_owned())
                .collect::<Vec<_>>(),
        );
        let selected = self.config.selected_version.as_ref().and_then(|selected| {
            self.installed_versions
                .iter()
                .position(|version| version.id.as_str() == selected)
                .map(|row| IndexPath::default().row(row))
        });
        self.version_select.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_index(selected, window, cx);
        });
    }

    fn sync_account_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = SearchableVec::new(
            self.accounts
                .iter()
                .map(AccountSelectItem::new)
                .collect::<Vec<_>>(),
        );
        let selected = self.config.selected_account.as_ref().and_then(|selected| {
            self.accounts
                .iter()
                .position(|account| {
                    account.username() == selected
                        && stored_account_kind(account) == self.config.selected_account_kind
                })
                .map(|row| IndexPath::default().row(row))
        });
        self.account_select.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_index(selected, window, cx);
        });
    }

    fn selected_version(&self) -> Option<&InstalledVersion> {
        let selected = self.config.selected_version.as_deref()?;
        self.installed_versions
            .iter()
            .find(|version| version.id.as_str() == selected)
    }

    fn selected_account(&self) -> Option<&Account> {
        let selected = self.config.selected_account.as_deref()?;
        self.accounts.iter().find(|account| {
            account.username() == selected
                && stored_account_kind(account) == self.config.selected_account_kind
        })
    }

    fn is_installed(&self, version_id: &str) -> bool {
        self.installed_versions
            .iter()
            .any(|version| version.id.as_str() == version_id)
    }

    fn navigate(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.page = page;
        if page == Page::Logs {
            self.logs_attention = false;
        }
        if page == Page::Downloads && self.catalog.manifest.is_none() && !self.catalog.loading {
            self.load_catalog(false, window, cx);
        }
        cx.notify();
    }

    fn load_catalog(&mut self, refresh: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.catalog.loading {
            return;
        }
        if !refresh && self.catalog.manifest.is_some() {
            return;
        }
        self.catalog.loading = true;
        self.catalog.error = None;
        cx.notify();

        let catalog = self.services.catalog.clone();
        let runtime = self.services.runtime.handle().clone();
        self._catalog_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = runtime
                .spawn(async move { catalog.manifest(refresh, CancellationToken::new()).await })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.catalog.loading = false;
                match result {
                    Ok(Ok(manifest)) => {
                        view.catalog.manifest = Some(manifest);
                        view.catalog.error = None;
                    }
                    Ok(Err(AppError::Cancelled)) => {}
                    Ok(Err(error)) => {
                        let message = error.to_string();
                        view.catalog.error = Some(message.clone());
                        window.push_notification(
                            Notification::error(message).title("版本列表加载失败"),
                            cx,
                        );
                    }
                    Err(error) => {
                        let message = format!("后台任务失败: {error}");
                        view.catalog.error = Some(message.clone());
                        window.push_notification(
                            Notification::error(message).title("版本列表加载失败"),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
    }

    fn start_install_from_manifest(
        &mut self,
        version_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let request = VanillaInstallRequest {
            minecraft_root: self.config.minecraft_root.clone(),
            version_id: version_id.clone(),
            platform: self.services.platform.clone(),
            source: VersionSource::Manifest,
        };
        self.start_install_task(request, version_id, window, cx);
    }

    fn start_repair(&mut self, version_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let request = VanillaInstallRequest {
            minecraft_root: self.config.minecraft_root.clone(),
            version_id: version_id.clone(),
            platform: self.services.platform.clone(),
            source: VersionSource::Existing,
        };
        self.start_install_task(request, version_id, window, cx);
    }

    fn start_install_task(
        &mut self,
        request: VanillaInstallRequest,
        version_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.log_state.can_start() {
            window.push_notification(
                Notification::warning("已有任务正在运行，请等待完成或先停止它。")
                    .title("无法开始安装"),
                cx,
            );
            return;
        }
        window.push_notification(
            Notification::info(format!("正在检查并安装 {version_id}。")).title("已开始安装"),
            cx,
        );
        self.spawn_command(Command::Install(request), version_id, window, cx);
    }

    fn start_launch(&mut self, version_id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.launch_resolving {
            return;
        }
        if !self.log_state.can_start() {
            window.push_notification(
                Notification::warning("已有任务正在运行，请等待完成或先停止它。")
                    .title("无法启动游戏"),
                cx,
            );
            return;
        }
        let Some(account) = self.selected_account().cloned() else {
            window.push_notification(
                Notification::error("请先添加一个账户。").title("无法启动游戏"),
                cx,
            );
            return;
        };
        match account {
            Account::Offline(account) => {
                self.spawn_launch(
                    LaunchAccount::from_offline(&account),
                    version_id,
                    window,
                    cx,
                );
            }
            Account::Microsoft(account) => {
                self.launch_resolving = true;
                cx.notify();
                let service = self.services.microsoft_accounts.clone();
                let runtime = self.services.runtime.handle().clone();
                let cancellation = CancellationToken::new();
                self._launch_task = Some(cx.spawn_in(window, async move |this, cx| {
                    let result = runtime
                        .spawn(async move {
                            service
                                .resolve_launch_account(&account, &cancellation)
                                .await
                        })
                        .await;
                    let _ = this.update_in(cx, move |view, window, cx| {
                        view.launch_resolving = false;
                        match result {
                            Ok(Ok(resolved)) => {
                                view.update_resolved_microsoft(&resolved, window, cx);
                                if let Some(warning) = resolved.persistence_warning.as_ref() {
                                    window.push_notification(
                                        Notification::warning(warning.clone()).title("账户"),
                                        cx,
                                    );
                                }
                                if !view.log_state.can_start() {
                                    window.push_notification(
                                        Notification::warning(
                                            "已有任务正在运行，请等待完成或先停止它。",
                                        )
                                        .title("无法启动游戏"),
                                        cx,
                                    );
                                } else {
                                    view.spawn_launch(resolved.launch, version_id, window, cx);
                                }
                            }
                            Ok(Err(error)) => {
                                window.push_notification(
                                    Notification::error(microsoft_error_message(&error))
                                        .title("无法启动游戏"),
                                    cx,
                                );
                            }
                            Err(error) => {
                                window.push_notification(
                                    Notification::error(format!("后台任务失败: {error}"))
                                        .title("无法启动游戏"),
                                    cx,
                                );
                            }
                        }
                        cx.notify();
                    });
                }));
            }
        }
    }

    fn spawn_launch(
        &mut self,
        account: LaunchAccount,
        version_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.log_state.can_start() {
            window.push_notification(
                Notification::warning("已有任务正在运行，请等待完成或先停止它。")
                    .title("无法启动游戏"),
                cx,
            );
            return;
        }
        let request = GameLaunchRequest {
            install: VanillaInstallRequest {
                minecraft_root: self.config.minecraft_root.clone(),
                version_id: version_id.clone(),
                platform: self.services.platform.clone(),
                source: VersionSource::Existing,
            },
            account,
            additional_jvm_arguments: Vec::new(),
            additional_game_arguments: Vec::new(),
            resolution: None,
            demo: false,
        };
        self.spawn_command(Command::Launch(request), version_id, window, cx);
    }

    fn update_resolved_microsoft(
        &mut self,
        resolved: &ResolvedMicrosoftAccount,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_microsoft_profile(resolved.profile.clone(), window, cx);
    }

    fn spawn_command(
        &mut self,
        command: Command,
        version_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(previous) = self.log_state.active_id()
            && self
                .log_state
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.status.is_terminal())
        {
            let manager = self.services.tasks.clone();
            self.services
                .runtime
                .spawn(async move { manager.dismiss(previous).await });
        }
        self.log_state.begin_command();
        self.logs_attention = false;
        self.active_task_version = Some(version_id);
        cx.notify();

        let services = self.services.clone();
        self._command_task = Some(cx.spawn_in(window, async move |this, cx| {
            let manager = services.tasks.clone();
            let command_manager = manager.clone();
            let result = services
                .runtime
                .spawn(async move {
                    match command {
                        Command::Install(request) => command_manager.start_install(request).await,
                        Command::Launch(request) => {
                            command_manager.start_launch_installed(request).await
                        }
                    }
                })
                .await;
            match result {
                Ok(Ok(id)) => {
                    let snapshot = manager.snapshot(id).await;
                    let logs = manager.logs(id).await;
                    let _ = this.update_in(cx, |view, window, cx| {
                        view.log_state.complete_start(id, snapshot.clone(), logs);
                        if let Some(snapshot) = snapshot
                            && snapshot.status.is_terminal()
                        {
                            view.on_task_terminal(snapshot, window, cx);
                        }
                        cx.notify();
                    });
                }
                Ok(Err(error)) => {
                    let message = error.to_string();
                    let _ = this.update_in(cx, |view, window, cx| {
                        view.log_state.fail_command(message.clone());
                        window.push_notification(
                            Notification::error(message).title("任务启动失败"),
                            cx,
                        );
                        cx.notify();
                    });
                }
                Err(error) => {
                    let message = format!("后台任务失败: {error}");
                    let _ = this.update_in(cx, |view, window, cx| {
                        view.log_state.fail_command(message.clone());
                        window.push_notification(
                            Notification::error(message).title("任务启动失败"),
                            cx,
                        );
                        cx.notify();
                    });
                }
            }
        }));
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.log_state.active_id() else {
            return;
        };
        if !self.log_state.can_cancel() {
            return;
        }
        self.log_state.begin_command();
        cx.notify();

        let services = self.services.clone();
        self._command_task = Some(cx.spawn_in(window, async move |this, cx| {
            let manager = services.tasks.clone();
            let cancel_manager = manager.clone();
            let result = services
                .runtime
                .spawn(async move { cancel_manager.cancel(id).await })
                .await;
            let snapshot = manager.snapshot(id).await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.log_state.finish_command();
                if let Some(snapshot) = snapshot {
                    view.log_state.apply_snapshot(snapshot);
                }
                match result {
                    Ok(_) => {
                        window.push_notification(
                            Notification::info("已请求停止当前任务。").title("正在停止"),
                            cx,
                        );
                    }
                    Err(error) => {
                        window.push_notification(
                            Notification::error(format!("取消命令失败: {error}"))
                                .title("无法停止任务"),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
    }

    fn hide_for_game(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        if self.launcher_hidden {
            return;
        }
        self.launcher_hidden = true;
        window.minimize_window();
    }

    fn restore_launcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.launcher_hidden {
            return;
        }
        self.launcher_hidden = false;
        #[cfg(target_os = "macos")]
        deminiaturize_windows();
        cx.activate(true);
        window.activate_window();
    }

    fn on_task_terminal(
        &mut self,
        snapshot: TaskSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.restore_launcher(window, cx);
        if !self.log_state.mark_terminal_notified(snapshot.id) {
            return;
        }
        let version = self.active_task_version.clone();
        match snapshot.status {
            TaskStatus::Completed => {
                if let Some(version) = version {
                    self.config.selected_version = Some(version);
                }
                self.reload_versions(window, cx);
                self.save_config();
                window.push_notification(
                    Notification::success("Minecraft 安装已完成，可以开始游戏了。")
                        .title("安装完成"),
                    cx,
                );
            }
            TaskStatus::Exited => {
                self.reload_versions(window, cx);
                let abnormal = !snapshot.stop_requested
                    && !snapshot.exit.as_ref().is_some_and(|exit| exit.success);
                if abnormal {
                    self.logs_attention = true;
                    let detail = snapshot
                        .exit
                        .as_ref()
                        .map(|exit| match (exit.code, exit.signal) {
                            (Some(code), _) => format!("退出码 {code}"),
                            (_, Some(signal)) => format!("信号 {signal}"),
                            _ => "未知退出状态".to_owned(),
                        })
                        .unwrap_or_else(|| "未知退出状态".to_owned());
                    window.push_notification(
                        Notification::warning(format!("游戏异常退出（{detail}）。"))
                            .title("游戏异常退出"),
                        cx,
                    );
                }
            }
            TaskStatus::Failed => {
                self.logs_attention = true;
                window.push_notification(
                    Notification::error(
                        snapshot
                            .error
                            .clone()
                            .unwrap_or_else(|| "任务失败，请查看日志。".to_owned()),
                    )
                    .title("任务失败"),
                    cx,
                );
            }
            TaskStatus::Cancelled => {
                window.push_notification(Notification::info("任务已取消。").title("已取消"), cx);
            }
            TaskStatus::Running | TaskStatus::Cancelling => {}
        }
        cx.notify();
    }

    fn browse_minecraft_root(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(target_os = "macos")]
        {
            match pick_directory_macos(&self.config.minecraft_root) {
                Ok(Some(path)) => self.set_minecraft_root(path, window, cx),
                Ok(None) => {}
                Err(error) => {
                    window.push_notification(Notification::error(error).title("选择目录失败"), cx)
                }
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let prompt = cx.prompt_for_paths(PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some("选择 Minecraft 目录".into()),
            });
            self._prompt_task = Some(cx.spawn_in(
                window,
                async move |this, cx| match prompt.await {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            let _ = this.update_in(cx, |view, window, cx| {
                                view.set_minecraft_root(path, window, cx)
                            });
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => {
                        let _ = this.update_in(cx, |_, window, cx| {
                            window.push_notification(
                                Notification::error(format!("无法打开目录选择器: {error}"))
                                    .title("选择目录失败"),
                                cx,
                            );
                        });
                    }
                    Err(_) => {
                        let _ = this.update_in(cx, |_, window, cx| {
                            window.push_notification(
                                Notification::error("目录选择器已取消或窗口已关闭。")
                                    .title("选择目录失败"),
                                cx,
                            );
                        });
                    }
                },
            ));
            cx.notify();
        }
    }

    fn set_minecraft_root(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let path = std::path::absolute(&path).unwrap_or(path);
        if path == self.config.minecraft_root {
            self.reload_versions(window, cx);
            return;
        }
        self.config.minecraft_root = path;
        self.save_config();
        self.reload_versions(window, cx);
        if let Some(error) = self.versions_error.clone() {
            window.push_notification(Notification::error(error).title("无法读取版本列表"), cx);
        } else {
            window.push_notification(
                Notification::success("游戏目录已更新。").title("设置已保存"),
                cx,
            );
        }
        cx.notify();
    }

    fn set_auto_java(&mut self, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.config.auto_install_java = enabled;
        self.save_config();
        window.push_notification(
            Notification::info("Java 自动获取设置将在下次启动 RMCL 时生效。").title("设置已保存"),
            cx,
        );
        cx.notify();
    }

    fn open_accounts_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let (accounts, heads, selected, selected_kind) = view
                .upgrade()
                .map(|view| {
                    let this = view.read(cx);
                    (
                        this.accounts.clone(),
                        this.account_heads.clone(),
                        this.config.selected_account.clone(),
                        this.config.selected_account_kind,
                    )
                })
                .unwrap_or_default();

            let mut list = v_flex()
                .w_full()
                .gap_1()
                .max_h(px(320.))
                .overflow_y_scrollbar();
            for (index, account) in accounts.iter().enumerate() {
                let username = account.username().to_owned();
                let kind = stored_account_kind(account);
                let is_selected =
                    selected.as_deref() == Some(username.as_str()) && selected_kind == kind;
                let subtitle = if kind == StoredAccountKind::Microsoft {
                    "正版账户"
                } else {
                    "离线账户"
                };
                let head = heads.get(&account.uuid());
                let row_view = view.clone();
                let row_username = username.clone();
                let delete_view = view.clone();
                let delete_account = account.clone();
                list = list.child(
                    h_flex()
                        .id(format!("account-row-{index}"))
                        .w_full()
                        .p_2()
                        .gap_3()
                        .rounded(cx.theme().radius)
                        .when(is_selected, |this| {
                            this.bg(cx.theme().primary.opacity(0.10))
                        })
                        .hover(|style| style.bg(cx.theme().accent.opacity(0.45)))
                        .on_click(move |_, window, cx| {
                            if let Some(view) = row_view.upgrade() {
                                view.update(cx, |this, cx| {
                                    this.select_account_key(&row_username, kind, window, cx);
                                    window.close_dialog(cx);
                                    cx.notify();
                                });
                            }
                        })
                        .child(account_avatar(Some(account), head, 32., cx))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_0p5()
                                .child(div().text_sm().font_medium().truncate().child(username))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(subtitle),
                                ),
                        )
                        .child(
                            Button::new(format!("account-delete-{index}"))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Trash)
                                .tooltip("删除账户")
                                .accessibility_label("删除账户")
                                .on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    if let Some(view) = delete_view.upgrade() {
                                        view.update(cx, |this, cx| {
                                            this.open_delete_account_dialog(
                                                delete_account.clone(),
                                                window,
                                                cx,
                                            )
                                        });
                                    }
                                }),
                        )
                        .when(is_selected, |this| {
                            this.child(
                                Icon::new(IconName::CircleCheck)
                                    .with_size(Size::Small)
                                    .text_color(cx.theme().primary),
                            )
                        }),
                );
            }
            if accounts.is_empty() {
                list = list.child(
                    div()
                        .w_full()
                        .py_8()
                        .text_center()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("还没有账户"),
                );
            }

            let login_view = view.clone();
            let offline_view = view.clone();
            dialog.title("账户").width(px(468.)).child(list).footer(
                DialogFooter::new()
                    .child(
                        Button::new("accounts-close")
                            .ghost()
                            .label("关闭")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("offline-add")
                            .outline()
                            .label("添加离线账户")
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                if let Some(view) = offline_view.upgrade() {
                                    view.update(cx, |this, cx| {
                                        this.open_offline_account_dialog(false, window, cx)
                                    });
                                }
                            }),
                    )
                    .child(
                        Button::new("microsoft-login")
                            .primary()
                            .label("正版登录")
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                if let Some(view) = login_view.upgrade() {
                                    view.update(cx, |this, cx| {
                                        this.begin_microsoft_login(window, cx)
                                    });
                                }
                            }),
                    ),
            )
        });
    }

    fn open_offline_account_dialog(
        &mut self,
        editing_existing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editing = if editing_existing {
            match self.selected_account() {
                Some(Account::Offline(account)) => Some(account.username().to_owned()),
                _ => self.accounts.iter().find_map(|account| match account {
                    Account::Offline(account) => Some(account.username().to_owned()),
                    Account::Microsoft(_) => None,
                }),
            }
        } else {
            None
        };
        let initial = editing.clone().unwrap_or_default();
        self.editing_account = editing.clone();
        self.account_input.update(cx, |state, cx| {
            state.set_value(initial.clone(), window, cx);
        });

        let input = self.account_input.clone();
        let view = cx.entity().downgrade();
        let title = if editing.is_some() {
            "编辑离线账户"
        } else {
            "添加离线账户"
        };
        let first_focus = Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if first_focus.replace(false) {
                input.read(cx).focus_handle(cx).focus(window, cx);
            }
            dialog
                .title(title)
                .child(
                    v_flex().gap_3().child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().font_medium().child("玩家名"))
                            .child(div().p_1().child(Input::new(&input).w_full())),
                    ),
                )
                .footer(
                    DialogFooter::new()
                        .child(DialogClose::new().trigger(|button| button.label("取消").outline()))
                        .child(
                            DialogAction::new()
                                .child(Button::new("save-account").primary().label("保存并切换")),
                        ),
                )
                .on_ok({
                    let view = view.clone();
                    let input = input.clone();
                    move |_, window, cx| {
                        let Some(view) = view.upgrade() else {
                            return true;
                        };
                        view.update(cx, |this, cx| {
                            let value = input.read(cx).value().to_string();
                            this.save_account(value, window, cx)
                        })
                    }
                })
        });
    }

    fn begin_microsoft_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_microsoft_login();
        self.microsoft_login = Some(MicrosoftLoginUi::Requesting);
        self.login_code_copied = false;
        let cancellation = CancellationToken::new();
        self.login_cancel = Some(cancellation.clone());
        self.spawn_microsoft_login(cancellation, window, cx);
        self.open_microsoft_login_dialog(window, cx);
        cx.notify();
    }

    fn spawn_microsoft_login(
        &mut self,
        cancellation: CancellationToken,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let service = self.services.microsoft_accounts.clone();
        let runtime = self.services.runtime.handle().clone();
        self._login_task = Some(cx.spawn_in(window, async move |this, cx| {
            let start = runtime
                .spawn({
                    let service = service.clone();
                    async move { service.start_device_login().await }
                })
                .await;
            let login = match start {
                Ok(Ok(login)) => login,
                Ok(Err(error)) => {
                    let message =
                        microsoft_error_message(&MicrosoftAccountServiceError::Auth(error));
                    let _ = this.update_in(cx, move |view, _, cx| {
                        view.fail_microsoft_login(message);
                        cx.notify();
                    });
                    return;
                }
                Err(error) => {
                    let message = format!("后台任务失败: {error}");
                    let _ = this.update_in(cx, move |view, _, cx| {
                        view.fail_microsoft_login(message);
                        cx.notify();
                    });
                    return;
                }
            };
            let user_code = login.user_code().to_owned();
            let verification_uri = login.verification_uri().to_owned();
            let started = this.update_in(cx, |view, _, cx| {
                if !matches!(view.microsoft_login, Some(MicrosoftLoginUi::Requesting)) {
                    return;
                }
                view.microsoft_login = Some(MicrosoftLoginUi::Waiting {
                    user_code,
                    verification_uri: verification_uri.clone(),
                });
                cx.open_url(&verification_uri);
                cx.notify();
            });
            if started.is_err() {
                return;
            }

            let wait = runtime
                .spawn(async move { login.wait(&cancellation).await })
                .await;
            match wait {
                Ok(Ok(authenticated)) => {
                    let completing = this.update_in(cx, |view, _, cx| {
                        if view.microsoft_login.is_none() {
                            return;
                        }
                        view.microsoft_login = Some(MicrosoftLoginUi::Completing);
                        cx.notify();
                    });
                    if completing.is_err() {
                        return;
                    }
                    let stored = runtime
                        .spawn({
                            let service = service.clone();
                            let authenticated = authenticated.clone();
                            async move { service.store_login(&authenticated).await }
                        })
                        .await;
                    match stored {
                        Ok(Ok(())) => {
                            let _ = this.update_in(cx, move |view, window, cx| {
                                if view.microsoft_login.is_none() {
                                    return;
                                }
                                view.microsoft_login = None;
                                view.login_cancel = None;
                                view.add_microsoft_account(authenticated.account, window, cx);
                                window.close_dialog(cx);
                                window.push_notification(
                                    Notification::success("正版账户已登录。").title("账户"),
                                    cx,
                                );
                                cx.notify();
                            });
                        }
                        Ok(Err(error)) => {
                            let message = microsoft_error_message(&error);
                            let _ = this.update_in(cx, move |view, _, cx| {
                                view.fail_microsoft_login(message);
                                cx.notify();
                            });
                        }
                        Err(error) => {
                            let message = format!("后台任务失败: {error}");
                            let _ = this.update_in(cx, move |view, _, cx| {
                                view.fail_microsoft_login(message);
                                cx.notify();
                            });
                        }
                    }
                }
                Ok(Err(error)) => {
                    if matches!(error, MicrosoftAuthError::Cancelled) {
                        return;
                    }
                    let message =
                        microsoft_error_message(&MicrosoftAccountServiceError::Auth(error));
                    let _ = this.update_in(cx, move |view, _, cx| {
                        view.fail_microsoft_login(message);
                        cx.notify();
                    });
                }
                Err(error) => {
                    let message = format!("后台任务失败: {error}");
                    let _ = this.update_in(cx, move |view, _, cx| {
                        view.fail_microsoft_login(message);
                        cx.notify();
                    });
                }
            }
        }));
    }

    fn open_microsoft_login_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let (state, code_copied) = view
                .upgrade()
                .map(|view| {
                    let this = view.read(cx);
                    (this.microsoft_login.clone(), this.login_code_copied)
                })
                .unwrap_or((None, false));
            let state = state.unwrap_or(MicrosoftLoginUi::Requesting);
            let close_view = view.clone();
            let retry_view = view.clone();
            let failed = matches!(state, MicrosoftLoginUi::Failed { .. });
            let content = match state.clone() {
                MicrosoftLoginUi::Requesting => v_flex()
                    .gap_3()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Spinner::new()
                                    .with_size(Size::Small)
                                    .color(cx.theme().primary),
                            )
                            .child(div().text_sm().child("正在获取登录代码…")),
                    )
                    .into_any_element(),
                MicrosoftLoginUi::Waiting {
                    user_code,
                    verification_uri,
                } => {
                    let copy_code = user_code.clone();
                    let copy_view = view.clone();
                    let open_uri = verification_uri.clone();
                    let verification_label = verification_uri
                        .trim_start_matches("https://")
                        .trim_start_matches("http://")
                        .trim_end_matches('/')
                        .to_owned();
                    v_flex()
                        .gap_4()
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!(
                                    "在浏览器中打开 {verification_label}，输入下方代码。"
                                )),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_medium()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("登录代码"),
                                )
                                .child(
                                    h_flex()
                                        .w_full()
                                        .gap_3()
                                        .p_3()
                                        .rounded(cx.theme().radius)
                                        .border_1()
                                        .border_color(cx.theme().border)
                                        .bg(cx.theme().group_box)
                                        .child(
                                            div()
                                                .flex_1()
                                                .text_2xl()
                                                .font_semibold()
                                                .font_family(cx.theme().mono_font_family.clone())
                                                .child(user_code),
                                        )
                                        .child({
                                            let button = Button::new("copy-login-code")
                                                .small()
                                                .icon(if code_copied {
                                                    IconName::CircleCheck
                                                } else {
                                                    IconName::Copy
                                                })
                                                .tooltip(if code_copied {
                                                    "已复制"
                                                } else {
                                                    "复制登录代码"
                                                })
                                                .accessibility_label(if code_copied {
                                                    "已复制"
                                                } else {
                                                    "复制登录代码"
                                                })
                                                .on_click(move |_, window, cx| {
                                                    cx.write_to_clipboard(
                                                        ClipboardItem::new_string(
                                                            copy_code.clone(),
                                                        ),
                                                    );
                                                    if let Some(view) = copy_view.upgrade() {
                                                        view.update(cx, |this, cx| {
                                                            this.show_login_code_copied(window, cx)
                                                        });
                                                    }
                                                });
                                            if code_copied {
                                                button.success()
                                            } else {
                                                button.ghost()
                                            }
                                        }),
                                ),
                        )
                        .child(
                            Button::new("open-login-page")
                                .primary()
                                .icon(IconName::ExternalLink)
                                .label("打开浏览器")
                                .w_full()
                                .on_click(move |_, _, cx| {
                                    cx.open_url(&open_uri);
                                }),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Spinner::new()
                                        .with_size(Size::XSmall)
                                        .color(cx.theme().muted_foreground),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("等待登录完成，完成后会自动继续。"),
                                ),
                        )
                        .into_any_element()
                }
                MicrosoftLoginUi::Completing => v_flex()
                    .gap_3()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Spinner::new()
                                    .with_size(Size::Small)
                                    .color(cx.theme().primary),
                            )
                            .child(div().text_sm().child("正在获取 Minecraft 账户…")),
                    )
                    .into_any_element(),
                MicrosoftLoginUi::Failed { message } => v_flex()
                    .gap_2()
                    .child(div().text_sm().text_color(cx.theme().danger).child(message))
                    .into_any_element(),
            };

            let footer = if failed {
                DialogFooter::new()
                    .child(
                        Button::new("close-microsoft-login")
                            .ghost()
                            .label("关闭")
                            .on_click({
                                let view = view.clone();
                                move |_, window, cx| {
                                    if let Some(view) = view.upgrade() {
                                        view.update(cx, |this, cx| {
                                            this.cancel_microsoft_login(window, cx)
                                        });
                                    }
                                }
                            }),
                    )
                    .child(
                        Button::new("retry-microsoft-login")
                            .primary()
                            .label("重试")
                            .on_click(move |_, window, cx| {
                                if let Some(view) = retry_view.upgrade() {
                                    view.update(cx, |this, cx| {
                                        this.clear_microsoft_login();
                                        window.close_dialog(cx);
                                        this.begin_microsoft_login(window, cx);
                                    });
                                }
                            }),
                    )
            } else {
                DialogFooter::new().child(
                    Button::new("cancel-microsoft-login")
                        .ghost()
                        .label("取消")
                        .on_click({
                            let view = view.clone();
                            move |_, window, cx| {
                                if let Some(view) = view.upgrade() {
                                    view.update(cx, |this, cx| {
                                        this.cancel_microsoft_login(window, cx)
                                    });
                                }
                            }
                        }),
                )
            };

            dialog
                .title("正版登录")
                .width(px(468.))
                .child(content)
                .footer(footer)
                .on_close(move |_, _, cx| {
                    if let Some(view) = close_view.upgrade() {
                        view.update(cx, |this, _| this.clear_microsoft_login());
                    }
                })
        });
    }

    fn clear_microsoft_login(&mut self) {
        self.microsoft_login = None;
        self.login_code_copied = false;
        self._copy_feedback_task = None;
        if let Some(cancellation) = self.login_cancel.take() {
            cancellation.cancel();
        }
    }

    fn refresh_account_heads(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pending = self
            .accounts
            .iter()
            .filter_map(|account| match account {
                Account::Microsoft(account) => account
                    .skin_url()
                    .map(|url| (account.uuid(), url.to_owned())),
                Account::Offline(_) => None,
            })
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return;
        }
        let skins = self.services.skins.clone();
        let runtime = self.services.runtime.handle().clone();
        self._skin_task = Some(cx.spawn_in(window, async move |this, cx| {
            for (uuid, url) in pending {
                let result = runtime
                    .spawn({
                        let skins = skins.clone();
                        async move { skins.ensure_head(&url).await }
                    })
                    .await;
                if let Ok(Ok(path)) = result {
                    let _ = this.update_in(cx, move |view, _, cx| {
                        view.account_heads.insert(uuid, path);
                        cx.notify();
                    });
                }
            }
        }));
    }

    /// Refreshes profiles for accounts that predate stored skin metadata.
    fn refresh_microsoft_profiles(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pending = self
            .accounts
            .iter()
            .filter_map(|account| match account {
                Account::Microsoft(account) if account.skin_url().is_none() => {
                    Some(account.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return;
        }
        let service = self.services.microsoft_accounts.clone();
        let runtime = self.services.runtime.handle().clone();
        self._profile_task = Some(cx.spawn_in(window, async move |this, cx| {
            for account in pending {
                let cancellation = CancellationToken::new();
                let result = runtime
                    .spawn({
                        let service = service.clone();
                        async move { service.refresh_profile(&account, &cancellation).await }
                    })
                    .await;
                if let Ok(Ok(profile)) = result {
                    let _ = this.update_in(cx, move |view, window, cx| {
                        view.apply_microsoft_profile(profile, window, cx);
                    });
                }
            }
        }));
    }

    fn apply_microsoft_profile(
        &mut self,
        profile: MicrosoftAccount,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let uuid = profile.uuid();
        let previous = self.accounts.iter().find_map(|account| match account {
            Account::Microsoft(account) if account.uuid() == uuid => Some((
                account.username().to_owned(),
                account.skin_url().map(str::to_owned),
            )),
            _ => None,
        });
        let previous_name = previous.as_ref().map(|(name, _)| name.clone());
        let previous_skin_url = previous.and_then(|(_, skin_url)| skin_url);
        if let Some(account) = self.accounts.iter_mut().find(
            |account| matches!(account, Account::Microsoft(account) if account.uuid() == uuid),
        ) {
            *account = Account::Microsoft(profile.clone());
        } else {
            self.accounts.push(Account::Microsoft(profile.clone()));
        }
        if previous_skin_url.as_deref() != profile.skin_url() {
            self.account_heads.remove(&uuid);
        }
        if self.config.selected_account_kind == StoredAccountKind::Microsoft
            && self.config.selected_account == previous_name
        {
            self.config.selected_account = Some(profile.username().to_owned());
        }
        self.save_config();
        self.sync_account_select(window, cx);
        self.refresh_account_heads(window, cx);
        cx.notify();
    }

    fn show_login_code_copied(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.login_code_copied = true;
        self._copy_feedback_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1600))
                .await;
            let _ = this.update_in(cx, |view, _, cx| {
                view.login_code_copied = false;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn cancel_microsoft_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_microsoft_login();
        window.close_dialog(cx);
        cx.notify();
    }

    fn fail_microsoft_login(&mut self, message: String) {
        if self.microsoft_login.is_none() {
            return;
        }
        self.microsoft_login = Some(MicrosoftLoginUi::Failed { message });
        self.login_cancel = None;
    }

    fn add_microsoft_account(
        &mut self,
        account: MicrosoftAccount,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let uuid = account.uuid();
        let previous_skin_url = self.accounts.iter().find_map(|existing| match existing {
            Account::Microsoft(existing) if existing.uuid() == uuid => {
                existing.skin_url().map(str::to_owned)
            }
            _ => None,
        });
        self.accounts.retain(
            |existing| !matches!(existing, Account::Microsoft(existing) if existing.uuid() == uuid),
        );
        if previous_skin_url.as_deref() != account.skin_url() {
            self.account_heads.remove(&uuid);
        }
        self.accounts.push(Account::Microsoft(account.clone()));
        self.config.selected_account = Some(account.username().to_owned());
        self.config.selected_account_kind = StoredAccountKind::Microsoft;
        self.save_config();
        self.sync_account_select(window, cx);
        self.refresh_account_heads(window, cx);
        cx.notify();
    }

    fn open_delete_account_dialog(
        &mut self,
        account: Account,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = account.username().to_owned();
        let is_microsoft = account.kind() == AccountKind::Microsoft;
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let clicked_account = account.clone();
            let confirm_view = view.clone();
            dialog
                .title("删除账户")
                .width(px(420.))
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .child(format!("确定要删除账户「{name}」吗？")),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(if is_microsoft {
                                    "删除后会同时退出该正版账户。"
                                } else {
                                    "此操作不会删除游戏文件。"
                                }),
                        ),
                )
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("cancel-delete-account")
                                .ghost()
                                .label("取消")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("confirm-delete-account")
                                .danger()
                                .label("删除")
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    if let Some(view) = confirm_view.upgrade() {
                                        view.update(cx, |this, cx| {
                                            this.delete_account(clicked_account.clone(), window, cx)
                                        });
                                    }
                                }),
                        ),
                )
        });
    }

    fn delete_account(&mut self, account: Account, window: &mut Window, cx: &mut Context<Self>) {
        let Account::Microsoft(account) = &account else {
            self.finish_delete_account(account, window, cx);
            return;
        };
        let account = account.clone();
        let service = self.services.microsoft_accounts.clone();
        let runtime = self.services.runtime.handle().clone();
        self._account_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = runtime
                .spawn({
                    let service = service.clone();
                    let account = account.clone();
                    async move { service.remove_credentials(&account).await }
                })
                .await;
            let _ = this.update_in(cx, move |view, window, cx| {
                match result {
                    Ok(Ok(())) => {
                        view.finish_delete_account(Account::Microsoft(account), window, cx)
                    }
                    Ok(Err(error)) => {
                        window.push_notification(
                            Notification::error(format!("无法删除系统钥匙串中的登录凭据: {error}"))
                                .title("无法删除账户"),
                            cx,
                        );
                    }
                    Err(error) => {
                        window.push_notification(
                            Notification::error(format!("后台任务失败: {error}"))
                                .title("无法删除账户"),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
    }

    fn finish_delete_account(
        &mut self,
        account: Account,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = account_key(&account);
        let deleting_selected = self
            .selected_account()
            .is_some_and(|selected| account_key(selected) == key);
        self.accounts
            .retain(|existing| account_key(existing) != key);
        self.account_heads.remove(&account.uuid());
        if deleting_selected {
            self.config.selected_account = self
                .accounts
                .first()
                .map(|account| account.username().to_owned());
            self.config.selected_account_kind = self
                .accounts
                .first()
                .map(stored_account_kind)
                .unwrap_or(StoredAccountKind::Offline);
        }
        self.save_config();
        self.sync_account_select(window, cx);
        window.push_notification(Notification::success("账户已删除。").title("账户"), cx);
        cx.notify();
    }

    fn save_account(&mut self, value: String, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let username = value.trim().to_owned();
        if username.is_empty() {
            window.push_notification(
                Notification::error("玩家名不能为空。").title("无法保存账户"),
                cx,
            );
            return false;
        }
        let account = match OfflineAccount::new(username.clone()) {
            Ok(account) => account,
            Err(error) => {
                window.push_notification(
                    Notification::error(error.to_string()).title("无法保存账户"),
                    cx,
                );
                return false;
            }
        };
        if let Some(previous) = self.editing_account.take()
            && previous != username
        {
            self.accounts.retain(
                |account| !matches!(account, Account::Offline(account) if account.username() == previous),
            );
        }
        self.accounts.retain(
            |account| !matches!(account, Account::Offline(account) if account.username() == username),
        );
        self.accounts.push(Account::Offline(account));
        self.config.selected_account = Some(username);
        self.config.selected_account_kind = StoredAccountKind::Offline;
        self.sync_account_select(window, cx);
        self.save_config();
        window.push_notification(Notification::success("离线账户已保存。").title("账户"), cx);
        cx.notify();
        true
    }

    fn open_install_dialog(
        &mut self,
        version: VersionSummary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = version.id.clone();
        let type_label = version_type_label(version.version_type.as_deref());
        let date = short_date(version.release_time.as_deref());
        let target = self.config.minecraft_root.display().to_string();
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            dialog
                .title("安装版本")
                .child(
                    v_flex()
                        .gap_3()
                        .child(
                            h_flex()
                                .gap_3()
                                .child(version_tile(IconName::Download, cx.theme().primary))
                                .child(
                                    v_flex()
                                        .gap_1()
                                        .child(div().text_lg().font_semibold().child(id.clone()))
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(format!("{type_label} · {date}")),
                                        ),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_xs().child("安装到"))
                                .child(
                                    div()
                                        .text_xs()
                                        .font_family(cx.theme().mono_font_family.clone())
                                        .text_color(cx.theme().muted_foreground)
                                        .child(target.clone()),
                                ),
                        ),
                )
                .footer(
                    DialogFooter::new()
                        .child(DialogClose::new().trigger(|button| button.label("取消").outline()))
                        .child(
                            DialogAction::new()
                                .child(Button::new("install-version").primary().label("安装")),
                        ),
                )
                .on_ok({
                    let id = id.clone();
                    let view = view.clone();
                    move |_, window, cx| {
                        let Some(view) = view.upgrade() else {
                            return true;
                        };
                        view.update(cx, |this, cx| {
                            this.start_install_from_manifest(id.clone(), window, cx)
                        });
                        true
                    }
                })
        });
    }

    fn copy_logs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = formatted_logs(&self.log_state.logs);
        if text.is_empty() {
            window.push_notification(Notification::info("当前没有可复制的日志。"), cx);
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        window.push_notification(
            Notification::success("日志已复制到剪贴板。").title("已复制"),
            cx,
        );
    }

    fn export_logs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.log_state.active_id() else {
            window.push_notification(Notification::info("当前没有任务日志。"), cx);
            return;
        };
        let directory = self.services.config_store.logs_dir();
        if let Err(error) = std::fs::create_dir_all(&directory) {
            window.push_notification(
                Notification::error(format!("无法创建日志目录: {error}")).title("导出失败"),
                cx,
            );
            return;
        }
        let path = directory.join(format!("rmcl-task-{}.log", id.get()));
        let mut text = String::new();
        if let Some(snapshot) = &self.log_state.snapshot {
            text.push_str(&format!(
                "RMCL task {} | {:?} | {:?}\n",
                snapshot.id.get(),
                snapshot.kind,
                snapshot.status
            ));
            if let Some(phase) = snapshot.phase {
                text.push_str(&format!("phase: {phase:?}\n"));
            }
            if let Some(error) = &snapshot.error {
                text.push_str(&format!("error: {error}\n"));
            }
            text.push_str("---\n");
        }
        text.push_str(&formatted_logs(&self.log_state.logs));
        text.push('\n');
        match std::fs::write(&path, text) {
            Ok(()) => window.push_notification(
                Notification::success(format!("已导出到 {}", path.display())).title("日志已导出"),
                cx,
            ),
            Err(error) => window.push_notification(
                Notification::error(format!("无法写入日志: {error}")).title("导出失败"),
                cx,
            ),
        }
    }

    fn open_logs_dir(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let directory = self.services.config_store.logs_dir();
        let _ = std::fs::create_dir_all(&directory);
        if let Err(error) = open_path(&directory) {
            window.push_notification(
                Notification::error(format!("无法打开日志目录: {error}")).title("打开失败"),
                cx,
            );
        }
    }

    fn clear_logs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(snapshot) = &self.log_state.snapshot
            && matches!(
                snapshot.status,
                TaskStatus::Running | TaskStatus::Cancelling
            )
        {
            window.push_notification(
                Notification::warning("请先停止正在运行的任务。").title("无法清除日志"),
                cx,
            );
            return;
        }
        if let Some(id) = self.log_state.active_id() {
            let manager = self.services.tasks.clone();
            self.services
                .runtime
                .spawn(async move { manager.dismiss(id).await });
        }
        self.log_state = LauncherState::new();
        self.active_task_version = None;
        self.log_scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    fn confirm_delete_version(
        &mut self,
        version_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .confirm()
                .title(format!("删除版本 {version_id}？"))
                .description("将删除整个版本目录；共享 libraries 和 assets 不受影响。")
                .ok_text("删除")
                .ok_variant(ButtonVariant::Danger)
                .on_ok({
                    let version_id = version_id.clone();
                    let view = view.clone();
                    move |_, window, cx| {
                        let Some(view) = view.upgrade() else {
                            return true;
                        };
                        view.update(cx, |this, cx| {
                            this.delete_version(version_id.clone(), window, cx)
                        });
                        true
                    }
                })
        });
    }

    fn delete_version(&mut self, version_id: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.log_state.can_start() {
            window.push_notification(
                Notification::warning("请先等待当前任务结束。").title("无法删除版本"),
                cx,
            );
            return;
        }
        let display = version_id.clone();
        let root = self.config.minecraft_root.clone();
        let services = self.services.clone();
        self._command_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = services
                .runtime
                .spawn(async move { delete_installed_version(root, &version_id).await })
                .await;
            let _ = this.update_in(cx, |view, window, cx| match result {
                Ok(Ok(())) => {
                    view.reload_versions(window, cx);
                    view.save_config();
                    window.push_notification(
                        Notification::success(format!("已删除版本 {display}。"))
                            .title("版本已删除"),
                        cx,
                    );
                }
                Ok(Err(error)) => {
                    window.push_notification(
                        Notification::error(error.to_string()).title("删除版本失败"),
                        cx,
                    );
                }
                Err(error) => {
                    window.push_notification(
                        Notification::error(format!("后台任务失败: {error}")).title("删除版本失败"),
                        cx,
                    );
                }
            });
        }));
        cx.notify();
    }

    fn open_version_folder(
        &mut self,
        version_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(version) = self
            .installed_versions
            .iter()
            .find(|version| version.id.as_str() == version_id)
        else {
            return;
        };
        if let Err(error) = open_path(&version.version_dir) {
            window.push_notification(
                Notification::error(format!("无法打开版本目录: {error}")).title("打开失败"),
                cx,
            );
        }
    }

    fn open_game_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.config.minecraft_root.clone();
        let _ = std::fs::create_dir_all(&root);
        if let Err(error) = open_path(&root) {
            window.push_notification(
                Notification::error(format!("无法打开游戏目录: {error}")).title("打开失败"),
                cx,
            );
        }
    }

    fn refresh_versions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reload_versions(window, cx);
        self.save_config();
        if let Some(error) = self.versions_error.clone() {
            window.push_notification(Notification::error(error).title("无法读取版本列表"), cx);
        }
    }

    fn filtered_installed_versions(&self, cx: &App) -> Vec<InstalledVersion> {
        let query = self.versions_query.read(cx).value().to_lowercase();
        self.installed_versions
            .iter()
            .filter(|version| {
                self.versions_filter
                    .matches(version.version_type.as_deref())
                    && (query.is_empty() || version.id.as_str().to_lowercase().contains(&query))
            })
            .cloned()
            .collect()
    }

    fn filtered_remote_versions(&self, cx: &App) -> Vec<VersionSummary> {
        let query = self.downloads_query.read(cx).value().to_lowercase();
        let Some(manifest) = &self.catalog.manifest else {
            return Vec::new();
        };
        manifest
            .versions
            .iter()
            .filter(|version| {
                self.downloads_filter
                    .matches(version.version_type.as_deref())
                    && (query.is_empty() || version.id.to_lowercase().contains(&query))
            })
            .cloned()
            .collect()
    }

    fn latest_release(&self) -> Option<&str> {
        self.catalog
            .manifest
            .as_ref()
            .and_then(|manifest| manifest.latest.as_ref())
            .and_then(|latest| latest.release.as_deref())
    }
}

impl Render for LauncherApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = match self.page {
            Page::Home => self.render_home(cx).into_any_element(),
            Page::Versions => self.render_versions(cx).into_any_element(),
            Page::Downloads => self.render_downloads(cx).into_any_element(),
            Page::Logs => self.render_logs(cx).into_any_element(),
            Page::Settings => self.render_settings(window, cx).into_any_element(),
        };

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_title_bar(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .min_h_0()
                            .child(page)
                            .child(self.render_status_bar(cx)),
                    ),
            )
    }
}

pub fn platform_label(platform: &Platform) -> String {
    let os = match platform.os {
        OperatingSystem::Windows => "Windows",
        OperatingSystem::MacOs => "macOS",
        OperatingSystem::Linux => "Linux",
    };
    let arch = match platform.arch {
        Architecture::X86 => "x86",
        Architecture::X86_64 => "x86_64",
        Architecture::Arm32 => "arm32",
        Architecture::Aarch64 => "arm64",
        Architecture::Other(ref other) => other.as_str(),
    };
    format!("{os} · {arch}")
}

fn account_key(account: &Account) -> String {
    match account {
        Account::Offline(account) => format!("offline:{}", account.uuid()),
        Account::Microsoft(account) => format!("microsoft:{}", account.uuid()),
    }
}

fn account_avatar(
    account: Option<&Account>,
    head: Option<&PathBuf>,
    size: f32,
    cx: &App,
) -> AnyElement {
    if let Some(head) = head {
        return div()
            .size(px(size))
            .flex_shrink_0()
            .rounded_full()
            .overflow_hidden()
            .bg(cx.theme().primary.opacity(0.15))
            .child(img(head.clone()).size_full().object_fit(ObjectFit::Cover))
            .into_any_element();
    }
    let initial = account
        .and_then(|account| account.username().chars().next())
        .map(|character| character.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_owned());
    div()
        .size(px(size))
        .flex_shrink_0()
        .rounded_full()
        .bg(cx.theme().primary.opacity(0.15))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .text_sm()
                .font_semibold()
                .text_color(cx.theme().primary)
                .child(initial),
        )
        .into_any_element()
}

fn stored_account_kind(account: &Account) -> StoredAccountKind {
    match account.kind() {
        AccountKind::Offline => StoredAccountKind::Offline,
        AccountKind::Microsoft => StoredAccountKind::Microsoft,
    }
}

fn microsoft_error_message(error: &MicrosoftAccountServiceError) -> String {
    match error {
        MicrosoftAccountServiceError::CredentialsMissing
        | MicrosoftAccountServiceError::Auth(MicrosoftAuthError::ReauthenticationRequired)
        | MicrosoftAccountServiceError::Auth(MicrosoftAuthError::MinecraftTokenRejected) => {
            "正版账户登录已失效，请重新登录。".to_owned()
        }
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::NoMinecraftEntitlement) => {
            "该 Microsoft 账户未拥有 Minecraft: Java Edition。".to_owned()
        }
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::ProfileNotFound) => {
            "该账户还没有 Minecraft 玩家档案，请先在官方启动器登录一次。".to_owned()
        }
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::Xbox {
            code: Some(2148916233),
            ..
        }) => "该账户还没有 Xbox 档案，请先在 minecraft.net 登录一次。".to_owned(),
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::Xbox {
            code: Some(2148916238),
            ..
        }) => "该账户是儿童账户，需要先加入 Microsoft 家庭组。".to_owned(),
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::Xbox {
            code: Some(2148916235),
            ..
        }) => "该账户所在地区不支持 Xbox Live。".to_owned(),
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::Xbox {
            code: Some(2148916227),
            ..
        }) => "该账户已被 Xbox Live 封禁。".to_owned(),
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::DeviceCodeExpired) => {
            "登录代码已过期，请重新开始。".to_owned()
        }
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::DeviceCodeDeclined) => {
            "Microsoft 登录已取消。".to_owned()
        }
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::Cancelled) => {
            "Microsoft 登录已取消。".to_owned()
        }
        MicrosoftAccountServiceError::SecretStore(error) => {
            format!("无法访问系统钥匙串: {error}")
        }
        _ => error.to_string(),
    }
}

fn theme_index(theme: ThemePreference) -> usize {
    match theme {
        ThemePreference::System => 0,
        ThemePreference::Light => 1,
        ThemePreference::Dark => 2,
    }
}

fn visibility_index(visibility: LauncherVisibility) -> usize {
    match visibility {
        LauncherVisibility::KeepVisible => 0,
        LauncherVisibility::HideWhileRunning => 1,
    }
}

fn accent_index(accent: AccentColor) -> usize {
    match accent {
        AccentColor::Emerald => 0,
        AccentColor::Ocean => 1,
        AccentColor::Violet => 2,
        AccentColor::Amber => 3,
        AccentColor::Rose => 4,
    }
}

pub fn version_type_label(version_type: Option<&str>) -> &'static str {
    match version_type {
        Some("release") => "正式版",
        Some("snapshot") => "快照",
        Some("old_beta") => "旧版 Beta",
        Some("old_alpha") => "旧版 Alpha",
        Some(_) => "其他",
        None => "未知",
    }
}

pub fn short_date(value: Option<&str>) -> String {
    value
        .and_then(|value| value.get(..10))
        .filter(|value| value.len() == 10)
        .unwrap_or("日期未知")
        .to_owned()
}

pub fn formatted_logs(logs: &std::collections::VecDeque<GameLog>) -> String {
    logs.iter()
        .map(|log| {
            let stream = match log.stream {
                LogStream::Stdout => "out",
                LogStream::Stderr => "err",
            };
            format!("[{stream}] {}", log.text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn task_progress(snapshot: Option<&TaskSnapshot>) -> (f32, bool) {
    let Some(snapshot) = snapshot else {
        return (0.0, false);
    };
    let value = match snapshot.progress.as_ref() {
        Some(TaskProgress::Download(progress)) if progress.total_files > 0 => {
            progress.completed_files as f32 / progress.total_files as f32 * 100.0
        }
        Some(TaskProgress::Natives {
            completed_archives,
            total_archives,
            ..
        }) if *total_archives > 0 => *completed_archives as f32 / *total_archives as f32 * 100.0,
        _ if snapshot.status == TaskStatus::Completed => 100.0,
        _ => 0.0,
    };
    let indeterminate = matches!(
        snapshot.status,
        TaskStatus::Running | TaskStatus::Cancelling
    ) && value == 0.0;
    (value, indeterminate)
}

pub fn task_current_item(snapshot: Option<&TaskSnapshot>) -> String {
    let Some(progress) = snapshot.and_then(|snapshot| snapshot.progress.as_ref()) else {
        return String::new();
    };
    match progress {
        TaskProgress::Download(progress) => format!(
            "{} / {} 个文件 · {}",
            progress.completed_files,
            progress.total_files,
            progress.current.display()
        ),
        TaskProgress::Natives {
            current,
            completed_archives,
            total_archives,
        } => format!(
            "{completed_archives} / {total_archives} 个 native JAR · {}",
            current.display()
        ),
    }
}

pub fn task_phase_label(snapshot: Option<&TaskSnapshot>) -> String {
    let Some(phase) = snapshot.and_then(|snapshot| snapshot.phase) else {
        return "就绪".to_owned();
    };
    match phase {
        TaskPhase::Install(InstallPhase::ResolvingVersion) => "解析版本 manifest",
        TaskPhase::Install(InstallPhase::ReadingVersion) => "读取版本 JSON",
        TaskPhase::Install(InstallPhase::PersistingVersion) => "保存版本 JSON",
        TaskPhase::Install(InstallPhase::PlanningBase) => "规划游戏文件",
        TaskPhase::Install(InstallPhase::DownloadingBase) => "下载游戏文件",
        TaskPhase::Install(InstallPhase::ReadingAssetIndex) => "读取资源索引",
        TaskPhase::Install(InstallPhase::PlanningAssets) => "规划资源对象",
        TaskPhase::Install(InstallPhase::DownloadingAssets) => "下载资源对象",
        TaskPhase::Install(InstallPhase::MergingNatives) => "合并 natives",
        TaskPhase::Install(InstallPhase::Completed) => "安装已完成",
        TaskPhase::Inspecting => "检查版本文件",
        TaskPhase::CheckingJava => "检查 Java",
        TaskPhase::InstallingJava => "安装 Java",
        TaskPhase::PreparingLaunch => "准备启动参数",
        TaskPhase::StartingGame => "创建游戏进程",
        TaskPhase::RunningGame => "游戏运行中",
    }
    .to_owned()
}

pub fn task_status_label(snapshot: Option<&TaskSnapshot>) -> String {
    match snapshot.map(|snapshot| snapshot.status) {
        None => "就绪".to_owned(),
        Some(TaskStatus::Running) => "运行中".to_owned(),
        Some(TaskStatus::Cancelling) => "正在停止".to_owned(),
        Some(TaskStatus::Completed) => "安装完成".to_owned(),
        Some(TaskStatus::Cancelled) => "已取消".to_owned(),
        Some(TaskStatus::Failed) => "任务失败".to_owned(),
        Some(TaskStatus::Exited) => "游戏已退出".to_owned(),
    }
}

pub fn task_kind_label(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::VanillaInstall => "安装",
        TaskKind::GameLaunch => "启动",
    }
}

pub fn version_tile(icon: IconName, color: Hsla) -> impl IntoElement {
    div()
        .size(px(44.))
        .flex_shrink_0()
        .rounded(px(12.))
        .bg(color.opacity(0.12))
        .flex()
        .items_center()
        .justify_center()
        .child(Icon::new(icon).text_color(color).with_size(Size::Medium))
}

#[cfg(target_os = "macos")]
fn deminiaturize_windows() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    let Some(marker) = MainThreadMarker::new() else {
        return;
    };
    let application = NSApplication::sharedApplication(marker);
    for window in application.windows().iter() {
        if window.isMiniaturized() {
            window.deminiaturize(None);
        }
    }
}

#[cfg(target_os = "macos")]
fn pick_directory_macos(start: &std::path::Path) -> Result<Option<PathBuf>, String> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
    use objc2_foundation::{NSString, NSURL};

    let marker =
        MainThreadMarker::new().ok_or_else(|| "目录选择器必须在主线程打开。".to_owned())?;
    let panel = NSOpenPanel::openPanel(marker);
    panel.setCanChooseDirectories(true);
    panel.setCanChooseFiles(false);
    panel.setAllowsMultipleSelection(false);
    panel.setCanCreateDirectories(true);
    panel.setResolvesAliases(false);

    let initial = if start.is_dir() {
        Some(start)
    } else {
        start.parent().filter(|parent| parent.is_dir())
    };
    if let Some(initial) = initial
        && let Some(url) = NSURL::from_directory_path(initial)
    {
        panel.setDirectoryURL(Some(&url));
    }
    panel.setPrompt(Some(&NSString::from_str("选择")));
    if panel.runModal() != NSModalResponseOK {
        return Ok(None);
    }
    Ok(panel.URLs().iter().find_map(|url| url.to_file_path()))
}
