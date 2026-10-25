mod ui;

use std::{cell::Cell, path::PathBuf, rc::Rc, sync::Arc};

use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Icon, IndexPath, Sizable, Size, StyledExt, VirtualListScrollHandle, WindowExt,
        button::{Button, ButtonVariant, ButtonVariants},
        dialog::{DialogAction, DialogClose, DialogFooter},
        h_flex,
        input::{Input, InputEvent, InputState},
        notification::Notification,
        select::{SearchableVec, SelectEvent, SelectState},
        v_flex,
    },
    *,
};
use rmcl_app::{
    AppError, GameLaunchRequest, InstallPhase, InstalledVersion, TaskEvent, TaskKind, TaskPhase,
    TaskProgress, TaskSnapshot, TaskStatus, VanillaInstallRequest, VersionManifest, VersionSource,
    VersionSummary, delete_installed_version, discover_installed_versions,
};
use rmcl_auth::OfflineAccount;
use rmcl_domain::{Architecture, OperatingSystem, Platform};
use rmcl_launch::{GameLog, LogStream};
use rmcl_store::{
    AccentColor, CONFIG_SCHEMA_VERSION, LauncherConfig, LauncherVisibility, ThemePreference,
};
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

use crate::{
    services::{Bootstrap, Services, load_account, open_path, stored_account},
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

pub struct LauncherApp {
    services: Rc<Services>,
    page: Page,
    config: LauncherConfig,
    config_warning: Option<String>,
    config_save_error: Option<String>,
    installed_versions: Vec<InstalledVersion>,
    versions_error: Option<String>,
    accounts: Vec<OfflineAccount>,
    editing_account: Option<String>,
    version_select: Entity<SelectState<SearchableVec<String>>>,
    account_select: Entity<SelectState<SearchableVec<String>>>,
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
    _event_task: Task<()>,
    _command_task: Option<Task<()>>,
    _prompt_task: Option<Task<()>>,
    _catalog_task: Option<Task<()>>,
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
        let accounts: Vec<OfflineAccount> =
            config.accounts.iter().filter_map(load_account).collect();
        if !accounts
            .iter()
            .any(|account| Some(account.username()) == config.selected_account.as_deref())
        {
            config.selected_account = accounts
                .first()
                .map(|account| account.username().to_owned());
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
                .map(|account| account.username().to_owned())
                .collect::<Vec<_>>(),
        );
        let account_index = config.selected_account.as_ref().and_then(|selected| {
            accounts
                .iter()
                .position(|account| account.username() == selected)
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
            |this, _, event: &SelectEvent<SearchableVec<String>>, cx| {
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

        Self {
            services,
            page: Page::Home,
            config,
            config_warning,
            config_save_error: None,
            installed_versions,
            versions_error,
            accounts,
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
            downloads_filter: VersionFilter::All,
            versions_filter: VersionFilter::All,
            downloads_scroll: VirtualListScrollHandle::new(),
            active_task_version: None,
            launcher_hidden: false,
            logs_attention: false,
            _event_task: event_task,
            _command_task: None,
            _prompt_task: None,
            _catalog_task: None,
            _subscriptions: subscriptions,
            _appearance_subscription: appearance_subscription,
        }
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
        if self.config.selected_account.as_deref() == Some(value.as_str()) {
            return;
        }
        self.config.selected_account = Some(value);
        self.save_config();
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
        self.config.accounts = self.accounts.iter().map(stored_account).collect();
        if self.config.selected_account.is_none() {
            self.config.selected_account = self.accounts.first().map(|a| a.username().to_owned());
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
                .map(|account| account.username().to_owned())
                .collect::<Vec<_>>(),
        );
        let selected = self.config.selected_account.as_ref().and_then(|selected| {
            self.accounts
                .iter()
                .position(|account| account.username() == selected)
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

    fn selected_account(&self) -> Option<&OfflineAccount> {
        let selected = self.config.selected_account.as_deref()?;
        self.accounts
            .iter()
            .find(|account| account.username() == selected)
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
                Notification::error("请先添加一个离线账户。").title("无法启动游戏"),
                cx,
            );
            return;
        };
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
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择 Minecraft 目录".into()),
        });
        self._prompt_task = Some(
            cx.spawn_in(window, async move |this, cx| match prompt.await {
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
            }),
        );
        cx.notify();
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

    fn open_account_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editing = self.config.selected_account.clone().or_else(|| {
            self.accounts
                .first()
                .map(|account| account.username().to_owned())
        });
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
                    v_flex()
                        .gap_3()
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("UUID 由玩家名生成，不连接 Mojang 登录。"),
                        )
                        .child(
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
            self.accounts
                .retain(|account| account.username() != previous);
        }
        self.accounts
            .retain(|account| account.username() != username);
        self.accounts.push(account);
        self.config.selected_account = Some(username);
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = match self.page {
            Page::Home => self.render_home(cx).into_any_element(),
            Page::Versions => self.render_versions(cx).into_any_element(),
            Page::Downloads => self.render_downloads(cx).into_any_element(),
            Page::Logs => self.render_logs(cx).into_any_element(),
            Page::Settings => self.render_settings(cx).into_any_element(),
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
