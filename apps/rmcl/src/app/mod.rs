mod skin_service;
mod skin_viewer;
mod ui;

use std::{cell::Cell, collections::HashMap, path::PathBuf, rc::Rc, sync::Arc, time::Duration};

use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon, IndexPath, Sizable, Size, StyledExt,
        VirtualListScrollHandle, WindowExt,
        button::{Button, ButtonVariant, ButtonVariants},
        combobox::{ComboboxEvent, ComboboxState},
        dialog::{DialogAction, DialogClose, DialogFooter},
        h_flex,
        input::{Input, InputEvent, InputState},
        notification::Notification,
        scroll::ScrollableElement,
        searchable_list::SearchableListItem,
        select::{SearchableVec, SelectEvent, SelectState},
        spinner::Spinner,
        tag::Tag,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use rmcl_app::{
    AppError, ConflictPolicy, ContentGraphInstallRequest, ContentInstallPhase,
    ContentInstallPreflight, ContentKind, ContentLibrary, ContentPreflightItem,
    ContentPreflightStatus, ContentSource, CreateInstanceRequest, FeaturedServer,
    GameLaunchRequest, InstallPhase, InstalledVersion, InstanceCreatePhase, JavaRuntime,
    JavaRuntimeSource, LoaderKind, LoaderSelection, LoaderVersion, LocalContent,
    MicrosoftAccountServiceError, RemoteProject, RemoteSearchQuery, RemoteSort, RemoteVersion,
    RepositoryPath, ResolvedMicrosoftAccount, ServerIconCache, ServerPingError, ServerStatus,
    ServerStatusClient, TaskEvent, TaskKind, TaskPhase, TaskProgress, TaskSnapshot, TaskStatus,
    TaskSubject, UpdatePlan, UpdateProgress, UpdateRelease, VanillaInstallRequest, VersionManifest,
    VersionSource, VersionSummary, delete_installed_version, delete_instance_content,
    discover_installed_versions, featured_servers, import_instance_content,
    migrate_instance_to_isolated, preflight_content_install, recommended_version, rename_instance,
    scan_installed_mods, scan_instance_content, set_instance_content_enabled, set_instance_java,
    set_instance_launch_settings, spawn_apply_helper,
};
use rmcl_auth::{Account, AccountKind, MicrosoftAccount, MicrosoftAuthError, OfflineAccount};
use rmcl_domain::{Architecture, OperatingSystem, Platform, ServerAddress};
use rmcl_launch::{GameLog, LaunchAccount, LogStream};
use rmcl_net::{MirrorHealth, MirrorPreference, MirrorProbe};
use rmcl_store::{
    AccentColor, CONFIG_SCHEMA_VERSION, DownloadSourcePreference, LauncherConfig,
    LauncherVisibility, StoredAccountKind, StoredServer, ThemePreference,
};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
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

use self::{
    skin_service::{SkinService, SkinServiceEvent},
    skin_viewer::SkinViewer,
    ui::{
        layout,
        skin::{ApplyState, LinkImport, LinkImportState, SkinEditor},
    },
};

const THEME_ITEMS: [&str; 3] = ["跟随系统", "浅色", "深色"];
const ACCENT_ITEMS: [&str; 5] = ["翡翠", "海蓝", "紫罗兰", "琥珀", "玫瑰"];
const VISIBILITY_ITEMS: [&str; 2] = ["保持显示", "游戏运行时隐藏"];
const DOWNLOAD_SOURCE_ITEMS: [&str; 3] = ["自动", "官方源", "国内镜像"];
const SERVER_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const SERVER_PING_CONCURRENCY: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Home,
    Versions,
    Skin,
    Servers,
    Instance,
    CreateInstance,
    ContentBrowser,
    Downloads,
    Logs,
    Settings,
}

impl Page {
    pub fn title(self) -> &'static str {
        match self {
            Self::Home => "主页",
            Self::Versions => "实例",
            Self::Skin => "皮肤",
            Self::Servers => "联机",
            Self::Instance => "实例详情",
            Self::CreateInstance => "新建实例",
            Self::ContentBrowser => "在线内容",
            Self::Downloads => "下载游戏",
            Self::Logs => "日志",
            Self::Settings => "设置",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Self::Home => "选择一个实例，立即开始游戏。",
            Self::Versions => "管理已安装的游戏实例与版本。",
            Self::Skin => "查看并更换当前正版账户的官方皮肤。",
            Self::Servers => "",
            Self::Instance => "管理该实例的 Mod、资源包和光影。",
            Self::CreateInstance => "选择 Minecraft 版本、加载器和实例目录。",
            Self::ContentBrowser => "从 Modrinth 搜索并安装到当前实例。",
            Self::Downloads => "从 Mojang 官方源安装新的 Vanilla 版本。",
            Self::Logs => "查看安装、启动和游戏运行日志。",
            Self::Settings => "配置游戏目录、外观与 Java。",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Home => IconName::House,
            Self::Versions => IconName::Boxes,
            Self::Skin => IconName::Shirt,
            Self::Servers => IconName::Network,
            Self::Instance => IconName::Package,
            Self::CreateInstance => IconName::PackagePlus,
            Self::ContentBrowser => IconName::Globe,
            Self::Downloads => IconName::Download,
            Self::Logs => IconName::ScrollText,
            Self::Settings => IconName::Settings,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ServerTab {
    #[default]
    Favorites,
    Discover,
}

impl ServerTab {
    pub const ALL: [Self; 2] = [Self::Favorites, Self::Discover];

    pub fn label(self) -> &'static str {
        match self {
            Self::Favorites => "收藏",
            Self::Discover => "发现",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Favorites => 0,
            Self::Discover => 1,
        }
    }

    pub fn from_index(index: usize) -> Self {
        match index {
            0 => Self::Favorites,
            1 => Self::Discover,
            _ => unreachable!("invalid server tab index: {index}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContentTab {
    #[default]
    Mods,
    ResourcePacks,
    ShaderPacks,
}

#[derive(Default)]
pub struct ContentBrowserState {
    kind: Option<ContentKind>,
    project: Option<RemoteProject>,
    projects: Vec<RemoteProject>,
    total_hits: u64,
    search_loading: bool,
    search_error: Option<String>,
    versions: Vec<RemoteVersion>,
    versions_loading: bool,
    versions_error: Option<String>,
    recommended: Option<String>,
    preflighting: bool,
}

#[derive(Default)]
struct ServerStatusState {
    statuses: HashMap<String, ServerStatusView>,
    refreshing: bool,
    request: u64,
    cancel: Option<CancellationToken>,
}

#[derive(Default)]
struct ServersState {
    statuses: HashMap<String, ServerStatusView>,
    refreshing: bool,
    request: u64,
    cancel: Option<CancellationToken>,
    featured: ServerStatusState,
}

#[derive(Clone, Default)]
struct ServerStatusView {
    loading: bool,
    status: Option<ServerStatus>,
    error: Option<String>,
    icon_path: Option<PathBuf>,
}

#[derive(Default)]
struct UpdateState {
    checking: bool,
    downloading: bool,
    available: Option<UpdateRelease>,
    progress: Option<UpdateProgress>,
    error: Option<String>,
    up_to_date: bool,
}

impl ContentTab {
    pub const ALL: [Self; 3] = [Self::Mods, Self::ResourcePacks, Self::ShaderPacks];

    pub fn label(self) -> &'static str {
        self.kind().label()
    }

    pub fn kind(self) -> ContentKind {
        match self {
            Self::Mods => ContentKind::Mod,
            Self::ResourcePacks => ContentKind::ResourcePack,
            Self::ShaderPacks => ContentKind::ShaderPack,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Mods => 0,
            Self::ResourcePacks => 1,
            Self::ShaderPacks => 2,
        }
    }

    fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(Self::Mods)
    }

    pub fn icon(self) -> IconName {
        match self {
            Self::Mods => IconName::Package,
            Self::ResourcePacks => IconName::Image,
            Self::ShaderPacks => IconName::Sparkles,
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

/// One selectable base version in the create-instance version picker.
#[derive(Clone)]
pub struct CreateVersionChoice {
    summary: VersionSummary,
}

impl CreateVersionChoice {
    fn new(summary: VersionSummary) -> Self {
        Self { summary }
    }
}

impl SearchableListItem for CreateVersionChoice {
    type Value = String;

    fn title(&self) -> SharedString {
        self.summary.id.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.summary.id
    }

    fn matches(&self, query: &str) -> bool {
        self.summary
            .id
            .to_lowercase()
            .contains(&query.to_lowercase())
    }

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .gap_3()
            .child(div().text_sm().child(self.summary.id.clone()))
            .child(
                h_flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .child(ui::common::type_tag(self.summary.version_type.as_deref()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(short_date(self.summary.release_time.as_deref())),
                    ),
            )
    }
}

enum Command {
    Install(VanillaInstallRequest),
    CreateInstance(CreateInstanceRequest),
    ContentInstall(ContentGraphInstallRequest),
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

pub struct LauncherApp {
    services: Rc<Services>,
    page: Page,
    config: LauncherConfig,
    config_warning: Option<String>,
    config_save_error: Option<String>,
    installed_versions: Vec<InstalledVersion>,
    versions_error: Option<String>,
    selected_instance: Option<String>,
    content_tab: ContentTab,
    content_library: Option<ContentLibrary>,
    content_loading: bool,
    content_error: Option<String>,
    content_action_pending: bool,
    content_request: u64,
    content_query: Entity<InputState>,
    content_browser: ContentBrowserState,
    remote_query: Entity<InputState>,
    remote_sort: RemoteSort,
    content_browser_request: u64,
    content_browser_cancel: Option<CancellationToken>,
    _content_browser_task: Option<Task<()>>,
    servers: ServersState,
    server_tab: ServerTab,
    server_query: Entity<InputState>,
    discover_query: Entity<InputState>,
    server_name_input: Entity<InputState>,
    server_address_input: Entity<InputState>,
    _server_task: Option<Task<()>>,
    _featured_task: Option<Task<()>>,
    _server_auto_task: Option<Task<()>>,
    update: UpdateState,
    _update_task: Option<Task<()>>,
    _update_poll_task: Option<Task<()>>,
    create_base_input: Entity<InputState>,
    create_version_select: Entity<ComboboxState<SearchableVec<CreateVersionChoice>>>,
    create_name_input: Entity<InputState>,
    create_id_input: Entity<InputState>,
    create_loader: Option<LoaderKind>,
    create_loader_versions: Vec<LoaderVersion>,
    create_loader_version: Option<LoaderVersion>,
    create_loader_loading: bool,
    create_loader_error: Option<String>,
    create_loader_cancel: Option<CancellationToken>,
    create_loader_request: u64,
    /// Existing instance whose content is copied when creating a loader
    /// instance from the instance detail page.
    create_copy_source: Option<String>,
    java_dialog_loading: bool,
    java_dialog_error: Option<String>,
    java_dialog_runtimes: Vec<JavaRuntime>,
    java_dialog_target: Option<String>,
    java_path_input: Entity<InputState>,
    java_custom_probing: bool,
    launch_memory_input: Entity<InputState>,
    launch_width_input: Entity<InputState>,
    launch_height_input: Entity<InputState>,
    _java_task: Option<Task<()>>,
    _create_task: Option<Task<()>>,
    _content_task: Option<Task<()>>,
    accounts: Vec<Account>,
    account_heads: HashMap<Uuid, PathBuf>,
    editing_account: Option<String>,
    skin_service: Entity<SkinService>,
    skin_viewer: Entity<SkinViewer>,
    skin_editor: SkinEditor,
    skin_link: LinkImport,
    skin_apply: ApplyState,
    pending_skin_profile: Option<MicrosoftAccount>,
    _skin_service_observe: Subscription,
    _skin_service_events: Subscription,
    _skin_link_subscription: Subscription,
    version_select: Entity<SelectState<SearchableVec<String>>>,
    theme_select: Entity<SelectState<SearchableVec<String>>>,
    accent_select: Entity<SelectState<SearchableVec<String>>>,
    visibility_select: Entity<SelectState<SearchableVec<String>>>,
    download_source_select: Entity<SelectState<SearchableVec<String>>>,
    mirror_status: String,
    _mirror_probe_task: Option<Task<()>>,
    log_state: LauncherState,
    log_scroll: ScrollHandle,
    log_follow: bool,
    catalog: CatalogState,
    downloads_query: Entity<InputState>,
    versions_query: Entity<InputState>,
    instance_name_input: Entity<InputState>,
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
        let download_source_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(DOWNLOAD_SOURCE_ITEMS.map(str::to_owned).to_vec()),
                Some(IndexPath::default().row(download_source_index(config.download_source))),
                window,
                cx,
            )
        });
        let mirror_status = initial_mirror_status(config.download_source);
        let mirror_probe_task = spawn_mirror_probe(
            cx,
            services.runtime.handle().clone(),
            services.mirror.clone(),
            config.download_source,
        );

        let downloads_query =
            cx.new(|cx| InputState::new(window, cx).placeholder("搜索版本 ID，例如 1.21.1"));
        let versions_query = cx.new(|cx| InputState::new(window, cx).placeholder("搜索已安装实例"));
        let content_query = cx.new(|cx| InputState::new(window, cx).placeholder("搜索本地内容"));
        let remote_query =
            cx.new(|cx| InputState::new(window, cx).placeholder("搜索 Modrinth 项目"));
        let server_query = cx.new(|cx| InputState::new(window, cx).placeholder("搜索服务器"));
        let discover_query = cx.new(|cx| InputState::new(window, cx).placeholder("搜索精选服务器"));
        let server_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("留空使用服务器地址"));
        let server_address_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("例如 play.example.com:25565"));
        let create_base_input = cx.new(|cx| InputState::new(window, cx).placeholder("例如 1.21.1"));
        let create_version_select = cx.new(|cx| {
            ComboboxState::new(
                SearchableVec::<CreateVersionChoice>::new(Vec::new()),
                Vec::new(),
                window,
                cx,
            )
            .searchable(true)
        });
        let create_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("留空使用实例 ID"));
        let create_id_input = cx.new(|cx| InputState::new(window, cx).placeholder("留空自动生成"));
        let instance_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("留空使用实例 ID"));
        let java_path_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("例如 /Library/Java/.../bin/java"));
        let launch_memory_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("留空使用 JVM 默认值"));
        let launch_width_input = cx.new(|cx| InputState::new(window, cx).placeholder("1280"));
        let launch_height_input = cx.new(|cx| InputState::new(window, cx).placeholder("720"));
        let account_input = cx.new(|cx| InputState::new(window, cx).placeholder("离线玩家名"));
        let skin_link_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("粘贴 PNG 图片直链"));

        let skin_service = cx.new(|cx| SkinService::new(services.clone(), cx));
        let skin_viewer = cx.new(|cx| SkinViewer::new(services.runtime.handle().clone(), cx));

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
        subscriptions.push(
            cx.subscribe(&content_query, |_this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        );
        subscriptions.push(
            cx.subscribe(&remote_query, |_this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        );
        subscriptions.push(
            cx.subscribe(&server_query, |_this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        );
        subscriptions.push(
            cx.subscribe(&discover_query, |_this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        );
        subscriptions.push(
            cx.subscribe(&create_base_input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    if let Some(cancellation) = this.create_loader_cancel.take() {
                        cancellation.cancel();
                    }
                    this.create_loader_request = this.create_loader_request.wrapping_add(1);
                    this.create_loader_loading = false;
                    this.create_loader_versions.clear();
                    this.create_loader_version = None;
                    this.create_loader_error = None;
                    cx.notify();
                }
            }),
        );
        subscriptions.push(cx.subscribe_in(
            &create_version_select,
            window,
            |this, _, event: &ComboboxEvent<SearchableVec<CreateVersionChoice>>, window, cx| {
                let ComboboxEvent::Confirm(values) = event else {
                    return;
                };
                let Some(base) = values.first() else {
                    return;
                };
                if this.create_base_input.read(cx).value().trim() == base.as_str() {
                    return;
                }
                this.create_base_input
                    .update(cx, |state, cx| state.set_value(base.clone(), window, cx));
                if this.create_loader.is_some() {
                    this.fetch_loader_versions(window, cx);
                }
            },
        ));
        subscriptions.push(
            cx.subscribe(&create_name_input, |_this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        );
        subscriptions.push(
            cx.subscribe(&create_id_input, |_this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
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
        subscriptions.push(cx.subscribe(
            &download_source_select,
            |this, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    this.set_download_source(value.clone(), cx);
                }
            },
        ));

        let skin_service_observe = cx.observe(&skin_service, |_, _, cx| cx.notify());
        let skin_service_events =
            cx.subscribe(&skin_service, |this, _, event: &SkinServiceEvent, cx| {
                let SkinServiceEvent::ProfileLoaded(loaded) = event;
                this.pending_skin_profile = Some(loaded.profile.clone());
                cx.notify();
            });
        let skin_link_subscription =
            cx.subscribe(&skin_link_input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    if matches!(this.skin_link.state, LinkImportState::Failed(_)) {
                        this.skin_link.state = LinkImportState::Idle;
                    }
                    cx.notify();
                }
            });

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
            selected_instance: None,
            content_tab: ContentTab::Mods,
            content_library: None,
            content_loading: false,
            content_error: None,
            content_action_pending: false,
            content_request: 0,
            content_query,
            content_browser: ContentBrowserState::default(),
            remote_query,
            remote_sort: RemoteSort::Relevance,
            content_browser_request: 0,
            content_browser_cancel: None,
            _content_browser_task: None,
            servers: ServersState::default(),
            server_tab: ServerTab::default(),
            server_query,
            discover_query,
            server_name_input,
            server_address_input,
            _server_task: None,
            _featured_task: None,
            _server_auto_task: None,
            update: UpdateState::default(),
            _update_task: None,
            _update_poll_task: None,
            create_base_input,
            create_version_select,
            create_name_input,
            create_id_input,
            create_loader: None,
            create_loader_versions: Vec::new(),
            create_loader_version: None,
            create_loader_loading: false,
            create_loader_error: None,
            create_loader_cancel: None,
            create_loader_request: 0,
            create_copy_source: None,
            java_dialog_loading: false,
            java_dialog_error: None,
            java_dialog_runtimes: Vec::new(),
            java_dialog_target: None,
            java_path_input,
            java_custom_probing: false,
            launch_memory_input,
            launch_width_input,
            launch_height_input,
            _java_task: None,
            _create_task: None,
            _content_task: None,
            accounts,
            account_heads: HashMap::new(),
            editing_account: None,
            skin_service,
            skin_viewer,
            skin_editor: SkinEditor::default(),
            skin_link: LinkImport::new(skin_link_input),
            skin_apply: ApplyState::Idle,
            pending_skin_profile: None,
            _skin_service_observe: skin_service_observe,
            _skin_service_events: skin_service_events,
            _skin_link_subscription: skin_link_subscription,
            version_select,
            theme_select,
            accent_select,
            visibility_select,
            download_source_select,
            mirror_status,
            _mirror_probe_task: mirror_probe_task,
            log_state: LauncherState::new(),
            log_scroll: ScrollHandle::new(),
            log_follow: true,
            catalog: CatalogState::default(),
            downloads_query,
            versions_query,
            instance_name_input,
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
        app.refresh_servers(window, cx);
        app._server_auto_task = Some(app.spawn_server_auto_refresh(window, cx));
        app._update_poll_task = Some(app.spawn_update_poll(window, cx));
        app
    }

    fn spawn_server_auto_refresh(&self, window: &mut Window, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(SERVER_REFRESH_INTERVAL)
                    .await;
                let keep_running = this
                    .update_in(cx, |view, window, cx| {
                        match view.page {
                            Page::Home => view.refresh_servers(window, cx),
                            Page::Servers => view.refresh_active_servers(window, cx),
                            _ => {}
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep_running {
                    break;
                }
            }
        })
    }

    fn spawn_update_poll(&self, window: &mut Window, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            loop {
                let keep_running = this
                    .update_in(cx, |view, window, cx| {
                        if view.config.auto_check_updates {
                            view.check_for_updates(false, window, cx);
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep_running {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_secs(6 * 60 * 60))
                    .await;
            }
        })
    }

    fn check_for_updates(&mut self, manual: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.update.checking || self.update.downloading {
            return;
        }
        self.update.checking = true;
        self.update.up_to_date = false;
        self.update.error = None;
        cx.notify();

        let client = self.services.updates.clone();
        let runtime = self.services.runtime.handle().clone();
        let current = env!("CARGO_PKG_VERSION").to_owned();
        self._update_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = runtime
                .spawn(async move { client.check(&current, &CancellationToken::new()).await })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.update.checking = false;
                match result {
                    Ok(Ok(Some(release))) => {
                        let version = release.version.clone();
                        view.update.available = Some(release);
                        view.update.up_to_date = false;
                        if !manual {
                            window.push_notification(
                                Notification::info(format!(
                                    "发现新版本 v{version}，可在设置中下载并安装。"
                                ))
                                .title("更新"),
                                cx,
                            );
                        }
                    }
                    Ok(Ok(None)) => {
                        view.update.available = None;
                        view.update.up_to_date = true;
                    }
                    Ok(Err(error)) => {
                        if manual {
                            view.update.error = Some(error.to_string());
                        }
                    }
                    Err(error) => {
                        if manual {
                            view.update.error = Some(format!("后台任务失败: {error}"));
                        }
                    }
                }
                cx.notify();
            });
        }));
    }

    fn set_auto_check_updates(
        &mut self,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.config.auto_check_updates = enabled;
        self.save_config();
        if enabled {
            self.check_for_updates(true, window, cx);
        }
        cx.notify();
    }

    fn open_update_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(release) = self.update.available.clone() else {
            return;
        };
        if !updates_installable() {
            window.push_notification(
                Notification::warning("开发构建不执行自动更新；请使用发布包验证更新流程。")
                    .title("更新"),
                cx,
            );
            return;
        }
        let view = cx.entity().downgrade();
        let dialog_release = release.clone();
        window.open_dialog(cx, move |dialog, _window, cx| {
            let save_view = view.clone();
            let save_release = dialog_release.clone();
            dialog
                .title(format!("更新到 RMCL {}", dialog_release.version))
                .width(layout::rem_px(cx, layout::DIALOG_XLARGE))
                .child(
                    v_flex()
                        .gap_3()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Tag::primary()
                                        .child(format!("v{}", dialog_release.version))
                                        .small(),
                                )
                                .child(
                                    Tag::new()
                                        .outline()
                                        .child(dialog_release.asset.package.label())
                                        .small(),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(dialog_release.asset.name.clone()),
                                ),
                        )
                        .child(
                            div()
                                .max_h(rems(15.))
                                .overflow_y_scrollbar()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(
                                    dialog_release
                                        .notes
                                        .clone()
                                        .filter(|notes| !notes.trim().is_empty())
                                        .unwrap_or_else(|| "该版本没有提供更新说明。".to_owned()),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("下载后会校验 SHA-256，替换当前程序并自动重启。"),
                        ),
                )
                .footer(
                    DialogFooter::new()
                        .child(DialogClose::new().trigger(|button| button.label("取消").outline()))
                        .child(
                            DialogAction::new()
                                .child(Button::new("confirm-update").primary().label("下载并安装")),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    let Some(view) = save_view.upgrade() else {
                        return true;
                    };
                    view.update(cx, |this, cx| {
                        this.start_update_download(save_release.clone(), window, cx)
                    });
                    true
                })
        });
    }

    fn start_update_download(
        &mut self,
        release: UpdateRelease,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.update.downloading || self.update.checking {
            return;
        }
        if !updates_installable() {
            window.push_notification(
                Notification::warning("开发构建不执行自动更新；请使用发布包验证更新流程。")
                    .title("更新"),
                cx,
            );
            return;
        }
        if !self.can_start_operation() {
            window.push_notification(
                Notification::warning("请等待当前任务结束后再更新启动器。").title("更新"),
                cx,
            );
            return;
        }
        let Some(config_dir) = self.services.config_store.path().parent() else {
            return;
        };
        let update_dir = config_dir.join("cache").join("updates");
        self.update.downloading = true;
        self.update.progress = Some(UpdateProgress {
            downloaded: 0,
            total: release.asset.size,
        });
        self.update.error = None;
        cx.notify();

        let client = self.services.updates.clone();
        let runtime = self.services.runtime.handle().clone();
        let download_dir = update_dir.clone();
        let log_dir = update_dir;
        let (progress_sender, mut progress_receiver) = mpsc::unbounded_channel();
        let (done_sender, mut done_receiver) = mpsc::unbounded_channel();
        self._update_task = Some(cx.spawn_in(window, async move |this, cx| {
            let download_client = client.clone();
            let download_release = release.clone();
            runtime.spawn(async move {
                let result = download_client
                    .download(
                        &download_release,
                        &download_dir,
                        Some(progress_sender),
                        &CancellationToken::new(),
                    )
                    .await;
                let _ = done_sender.send(result);
            });
            loop {
                tokio::select! {
                    Some(progress) = progress_receiver.recv() => {
                        let _ = this.update_in(cx, |view, _, cx| {
                            view.update.progress = Some(progress);
                            cx.notify();
                        });
                    }
                    Some(result) = done_receiver.recv() => {
                        let _ = this.update_in(cx, |view, window, cx| {
                            view.update.downloading = false;
                            match result {
                                Ok(archive) => {
                                    match UpdatePlan::prepare(&release, &archive, &log_dir) {
                                        Ok(plan) => match spawn_apply_helper(&plan) {
                                            Ok(()) => {
                                                window.push_notification(
                                                    Notification::info("RMCL 将退出并完成更新。")
                                                        .title("正在更新"),
                                                    cx,
                                                );
                                                cx.quit();
                                            }
                                            Err(error) => {
                                                view.update.error = Some(error.to_string());
                                            }
                                        },
                                        Err(error) => {
                                            view.update.error = Some(error.to_string());
                                        }
                                    }
                                }
                                Err(error) => {
                                    view.update.error = Some(error.to_string());
                                }
                            }
                            cx.notify();
                        });
                        break;
                    }
                    else => break,
                }
            }
        }));
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
                                .update_in(cx, |view, window, cx| {
                                    let was_terminal =
                                        view.log_state.snapshot.as_ref().is_some_and(|current| {
                                            current.id == snapshot.id
                                                && current.status.is_terminal()
                                        });
                                    let terminal_snapshot = snapshot.clone();
                                    view.log_state.recover(snapshot, logs.unwrap_or_default());
                                    if !was_terminal && terminal_snapshot.status.is_terminal() {
                                        view.on_task_terminal(terminal_snapshot, window, cx);
                                    }
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

    fn select_account_key(
        &mut self,
        username: &str,
        kind: StoredAccountKind,
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
        if self.page == Page::Skin {
            self.skin_editor.draft = None;
            self.skin_apply = ApplyState::Idle;
            self.skin_viewer
                .update(cx, |viewer, cx| viewer.reset_view(cx));
            self.ensure_skin_page(cx);
        }
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

    fn set_download_source(&mut self, value: String, cx: &mut Context<Self>) {
        let source = match value.as_str() {
            "自动" => DownloadSourcePreference::Auto,
            "官方源" => DownloadSourcePreference::Official,
            "国内镜像" => DownloadSourcePreference::Mirror,
            _ => return,
        };
        if self.config.download_source == source {
            return;
        }
        self.config.download_source = source;
        self.services
            .mirror
            .set_preference(mirror_preference(source));
        self.mirror_status = initial_mirror_status(source);
        self._mirror_probe_task = spawn_mirror_probe(
            cx,
            self.services.runtime.handle().clone(),
            self.services.mirror.clone(),
            source,
        );
        self.save_config();
        cx.notify();
    }

    fn retry_mirror_probe(&mut self, cx: &mut Context<Self>) {
        if self.config.download_source == DownloadSourcePreference::Official {
            return;
        }
        self.services
            .mirror
            .set_preference(mirror_preference(self.config.download_source));
        self.mirror_status = initial_mirror_status(self.config.download_source);
        self._mirror_probe_task = spawn_mirror_probe(
            cx,
            self.services.runtime.handle().clone(),
            self.services.mirror.clone(),
            self.config.download_source,
        );
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

    fn can_start_operation(&self) -> bool {
        self.log_state.can_start() && !self.content_operations_pending()
    }

    fn server_join_ready(&self) -> (bool, &'static str) {
        if self.selected_version().is_none() {
            (false, "请先选择一个实例")
        } else if self.selected_account().is_none() {
            (false, "请先添加一个账户")
        } else if !self.can_start_operation() {
            (false, "当前有任务或游戏正在运行，请先停止")
        } else {
            (true, "使用当前实例和账户进入服务器")
        }
    }

    fn content_operations_pending(&self) -> bool {
        self.content_action_pending
            || self.content_loading
            || self.log_state.has_running_content_task()
    }

    fn cancel_server_refresh(&mut self) {
        if let Some(cancellation) = self.servers.cancel.take() {
            cancellation.cancel();
        }
        self.servers.request = self.servers.request.wrapping_add(1);
        self.servers.refreshing = false;
        for status in self.servers.statuses.values_mut() {
            status.loading = false;
        }
        if let Some(cancellation) = self.servers.featured.cancel.take() {
            cancellation.cancel();
        }
        self.servers.featured.request = self.servers.featured.request.wrapping_add(1);
        self.servers.featured.refreshing = false;
        for status in self.servers.featured.statuses.values_mut() {
            status.loading = false;
        }
    }

    fn select_server_tab(&mut self, tab: ServerTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.server_tab == tab {
            return;
        }
        self.server_tab = tab;
        self.cancel_server_refresh();
        self.refresh_active_servers(window, cx);
        cx.notify();
    }

    fn refresh_active_servers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.server_tab {
            ServerTab::Favorites => self.refresh_servers(window, cx),
            ServerTab::Discover => self.refresh_featured_servers(window, cx),
        }
    }

    fn refresh_servers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let visible = match self.page {
            Page::Home => true,
            Page::Servers => self.server_tab == ServerTab::Favorites,
            _ => false,
        };
        if !visible || self.servers.refreshing {
            return;
        }
        let servers = self.config.servers.clone();
        self.servers
            .statuses
            .retain(|id, _| servers.iter().any(|server| server.id.as_str() == id));
        if servers.is_empty() {
            self.servers.statuses.clear();
            cx.notify();
            return;
        }

        if let Some(cancellation) = self.servers.cancel.take() {
            cancellation.cancel();
        }
        self.servers.request = self.servers.request.wrapping_add(1);
        let request = self.servers.request;
        let cancellation = CancellationToken::new();
        self.servers.cancel = Some(cancellation.clone());
        self.servers.refreshing = true;

        let mut entries = Vec::new();
        for server in &servers {
            let status = self.servers.statuses.entry(server.id.clone()).or_default();
            status.loading = true;
            match ServerAddress::parse(&server.address) {
                Ok(address) => entries.push((server.id.clone(), address)),
                Err(error) => {
                    status.loading = false;
                    status.status = None;
                    status.error = Some(format!("地址无效：{error}"));
                }
            }
        }
        if entries.is_empty() {
            self.servers.refreshing = false;
            cx.notify();
            return;
        }

        let client = self.services.servers.clone();
        let icons = self.services.server_icons.clone();
        let runtime = self.services.runtime.handle().clone();
        let entry_ids = entries.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
        self._server_task = Some(cx.spawn_in(window, async move |this, cx| {
            let results = runtime
                .spawn(async move { ping_servers(client, icons, entries, cancellation).await })
                .await;
            let _ = this.update_in(cx, |view, _, cx| {
                if view.servers.request != request {
                    return;
                }
                view.servers.refreshing = false;
                match results {
                    Ok(results) => {
                        for (id, result) in results {
                            if !view.config.servers.iter().any(|server| server.id == id) {
                                continue;
                            }
                            let status = view.servers.statuses.entry(id).or_default();
                            status.loading = false;
                            match result {
                                Ok((status_value, icon_path)) => {
                                    status.status = Some(status_value);
                                    if icon_path.is_some() {
                                        status.icon_path = icon_path;
                                    }
                                    status.error = None;
                                }
                                Err(error) => {
                                    status.status = None;
                                    status.error = Some(error.to_string());
                                }
                            }
                        }
                    }
                    Err(error) => {
                        for id in entry_ids {
                            let status = view.servers.statuses.entry(id).or_default();
                            status.loading = false;
                            status.status = None;
                            status.error = Some(format!("后台任务失败：{error}"));
                        }
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn refresh_featured_servers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::Servers
            || self.server_tab != ServerTab::Discover
            || self.servers.featured.refreshing
        {
            return;
        }
        let Ok(servers) = featured_servers() else {
            return;
        };
        self.servers
            .featured
            .statuses
            .retain(|id, _| servers.iter().any(|server| server.id == *id));
        if servers.is_empty() {
            self.servers.featured.statuses.clear();
            cx.notify();
            return;
        }

        if let Some(cancellation) = self.servers.featured.cancel.take() {
            cancellation.cancel();
        }
        self.servers.featured.request = self.servers.featured.request.wrapping_add(1);
        let request = self.servers.featured.request;
        let cancellation = CancellationToken::new();
        self.servers.featured.cancel = Some(cancellation.clone());
        self.servers.featured.refreshing = true;

        let mut entries = Vec::with_capacity(servers.len());
        for server in servers {
            let status = self
                .servers
                .featured
                .statuses
                .entry(server.id.clone())
                .or_default();
            status.loading = true;
            status.error = None;
            entries.push((server.id.clone(), server.address.clone()));
        }

        let client = self.services.servers.clone();
        let icons = self.services.server_icons.clone();
        let runtime = self.services.runtime.handle().clone();
        let entry_ids = entries.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
        self._featured_task = Some(cx.spawn_in(window, async move |this, cx| {
            let results = runtime
                .spawn(async move { ping_servers(client, icons, entries, cancellation).await })
                .await;
            let _ = this.update_in(cx, |view, _, cx| {
                if view.servers.featured.request != request {
                    return;
                }
                view.servers.featured.refreshing = false;
                match results {
                    Ok(results) => {
                        for (id, result) in results {
                            let Some(status) = view.servers.featured.statuses.get_mut(&id) else {
                                continue;
                            };
                            status.loading = false;
                            match result {
                                Ok((status_value, icon_path)) => {
                                    status.status = Some(status_value);
                                    if icon_path.is_some() {
                                        status.icon_path = icon_path;
                                    }
                                    status.error = None;
                                }
                                Err(error) => {
                                    status.status = None;
                                    status.error = Some(error.to_string());
                                }
                            }
                        }
                    }
                    Err(error) => {
                        for id in entry_ids {
                            if let Some(status) = view.servers.featured.statuses.get_mut(&id) {
                                status.loading = false;
                                status.status = None;
                                status.error = Some(format!("后台任务失败：{error}"));
                            }
                        }
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn featured_server_is_favorite(&self, address: &ServerAddress) -> bool {
        self.config.servers.iter().any(|stored| {
            ServerAddress::parse(&stored.address)
                .map(|stored_address| stored_address == *address)
                .unwrap_or(false)
        })
    }

    fn toggle_featured_server_favorite(
        &mut self,
        server: FeaturedServer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let saved_index = self.config.servers.iter().position(|stored| {
            ServerAddress::parse(&stored.address)
                .map(|stored_address| stored_address == server.address)
                .unwrap_or(false)
        });
        if let Some(index) = saved_index {
            self.config.servers.remove(index);
            self.save_config();
            cx.notify();
            window.push_notification(Notification::info("已取消收藏。").title("服务器"), cx);
            return;
        }
        self.config.servers.push(StoredServer {
            id: Uuid::new_v4().to_string(),
            name: server.name,
            address: server.address.to_string(),
        });
        self.save_config();
        cx.notify();
        window.push_notification(
            Notification::success("已加入服务器收藏。").title("服务器"),
            cx,
        );
    }

    fn join_featured_server(
        &mut self,
        server: FeaturedServer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if server.requires_mods {
            window.push_notification(
                Notification::warning("该服务器需要客户端 Mod 或整合包，请先准备对应实例。")
                    .title("需要整合包"),
                cx,
            );
            return;
        }
        let Some(version) = self.selected_version() else {
            window.push_notification(
                Notification::warning("请先在主页或实例页选择一个实例。").title("无法进入服务器"),
                cx,
            );
            return;
        };
        let version_id = version.id.as_str().to_owned();
        if self.selected_account().is_none() {
            window.push_notification(
                Notification::warning("请先添加一个账户。").title("无法进入服务器"),
                cx,
            );
            return;
        }
        if !self.can_start_operation() {
            window.push_notification(
                Notification::warning("已有任务或内容操作正在运行，请等待完成或先停止它。")
                    .title("无法进入服务器"),
                cx,
            );
            return;
        }
        self.start_launch_with_server(version_id, Some(server.address), window, cx);
    }

    fn open_server_dialog(
        &mut self,
        server_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = server_id
            .as_deref()
            .and_then(|id| self.config.servers.iter().find(|server| server.id == id))
            .cloned();
        let name = existing
            .as_ref()
            .map(|server| server.name.clone())
            .unwrap_or_default();
        let address = existing
            .as_ref()
            .map(|server| server.address.clone())
            .unwrap_or_default();
        self.server_name_input.update(cx, |state, cx| {
            state.set_value(name, window, cx);
        });
        self.server_address_input.update(cx, |state, cx| {
            state.set_value(address, window, cx);
        });

        let name_input = self.server_name_input.clone();
        let address_input = self.server_address_input.clone();
        let view = cx.entity().downgrade();
        let editing_id = existing.map(|server| server.id);
        let is_edit = editing_id.is_some();
        let first_focus = Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if first_focus.replace(false) {
                name_input.read(cx).focus_handle(cx).focus(window, cx);
            }
            let save_view = view.clone();
            let save_name = name_input.clone();
            let save_address = address_input.clone();
            let save_id = editing_id.clone();
            dialog
                .title(if is_edit { "编辑服务器" } else { "添加服务器" })
                .width(layout::rem_px(cx, layout::DIALOG_LARGE))
                .child(
                    v_flex()
                        .gap_4()
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_sm().font_medium().child("名称"))
                                .child(div().child(Input::new(&name_input).w_full()))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("留空时使用服务器地址。"),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_sm().font_medium().child("地址"))
                                .child(div().child(Input::new(&address_input).w_full()))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(
                                            "支持 host、host:port 和 [IPv6]:port；省略端口时使用 25565。",
                                        ),
                                ),
                        ),
                )
                .footer(
                    DialogFooter::new()
                        .child(DialogClose::new().trigger(|button| button.label("取消").outline()))
                        .child(
                            DialogAction::new()
                                .child(Button::new("save-server").primary().label("保存")),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    let Some(view) = save_view.upgrade() else {
                        return true;
                    };
                    view.update(cx, |this, cx| {
                        this.save_server(
                            save_id.clone(),
                            save_name.read(cx).value().to_string(),
                            save_address.read(cx).value().to_string(),
                            window,
                            cx,
                        )
                    })
                })
        });
    }

    fn save_server(
        &mut self,
        id: Option<String>,
        name: String,
        address: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let address = address.trim();
        if address.is_empty() {
            window.push_notification(
                Notification::error("服务器地址不能为空。").title("无法保存服务器"),
                cx,
            );
            return false;
        }
        let parsed = match ServerAddress::parse(address) {
            Ok(parsed) => parsed,
            Err(error) => {
                window.push_notification(
                    Notification::error(error.to_string()).title("无法保存服务器"),
                    cx,
                );
                return false;
            }
        };
        let name = name.trim();
        let name = if name.is_empty() {
            parsed.to_string()
        } else {
            name.to_owned()
        };
        match id {
            Some(id) => {
                let Some(server) = self
                    .config
                    .servers
                    .iter_mut()
                    .find(|server| server.id == id)
                else {
                    window.push_notification(
                        Notification::error("服务器不存在或已被删除。").title("无法保存服务器"),
                        cx,
                    );
                    return false;
                };
                server.name = name;
                server.address = address.to_owned();
            }
            None => self.config.servers.push(StoredServer {
                id: Uuid::new_v4().to_string(),
                name,
                address: address.to_owned(),
            }),
        }
        self.cancel_server_refresh();
        self.save_config();
        self.refresh_servers(window, cx);
        cx.notify();
        window.push_notification(Notification::success("服务器已保存。").title("服务器"), cx);
        true
    }

    fn confirm_delete_server(
        &mut self,
        server_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(server) = self
            .config
            .servers
            .iter()
            .find(|server| server.id == server_id)
            .cloned()
        else {
            return;
        };
        let view = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .confirm()
                .title(format!("删除服务器 {}？", server.name))
                .description("只删除 RMCL 中的收藏，不会修改游戏内的 servers.dat。")
                .ok_text("删除")
                .ok_variant(ButtonVariant::Danger)
                .on_ok({
                    let server_id = server_id.clone();
                    let view = view.clone();
                    move |_, window, cx| {
                        let Some(view) = view.upgrade() else {
                            return true;
                        };
                        view.update(cx, |this, cx| {
                            this.delete_server(server_id.clone(), window, cx)
                        });
                        true
                    }
                })
        });
    }

    fn delete_server(&mut self, server_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_server_refresh();
        self.config.servers.retain(|server| server.id != server_id);
        self.servers.statuses.remove(&server_id);
        self.save_config();
        self.refresh_servers(window, cx);
        cx.notify();
        window.push_notification(Notification::info("服务器已删除。").title("服务器"), cx);
    }

    fn join_server(&mut self, server_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(server) = self
            .config
            .servers
            .iter()
            .find(|server| server.id == server_id)
            .cloned()
        else {
            return;
        };
        let address = match ServerAddress::parse(&server.address) {
            Ok(address) => address,
            Err(error) => {
                window.push_notification(
                    Notification::error(error.to_string()).title("无法进入服务器"),
                    cx,
                );
                return;
            }
        };
        let Some(version) = self.selected_version() else {
            window.push_notification(
                Notification::warning("请先在主页或实例页选择一个实例。").title("无法进入服务器"),
                cx,
            );
            return;
        };
        self.start_launch_with_server(version.id.as_str().to_owned(), Some(address), window, cx);
    }

    fn navigate(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        let left_servers = self.page == Page::Servers && page != Page::Servers;
        self.page = page;
        if page != Page::CreateInstance {
            self.create_copy_source = None;
        }
        if page == Page::Versions {
            self.reload_versions(window, cx);
        }
        if left_servers {
            self.cancel_server_refresh();
        }
        if page == Page::Logs {
            self.logs_attention = false;
        }
        if page == Page::Downloads && self.catalog.manifest.is_none() && !self.catalog.loading {
            self.load_catalog(false, window, cx);
        }
        if page == Page::Home {
            self.refresh_servers(window, cx);
        }
        if page == Page::Servers {
            self.refresh_active_servers(window, cx);
        }
        if page == Page::Skin {
            self.ensure_skin_page(cx);
            self.skin_viewer
                .update(cx, |viewer, cx| viewer.reset_view(cx));
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
                        view.populate_create_version_choices(window, cx);
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
        if !self.can_start_operation() {
            window.push_notification(
                Notification::warning("已有任务或内容操作正在运行，请等待完成或先停止它。")
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

    fn start_content_install_task(
        &mut self,
        request: ContentGraphInstallRequest,
        instance_id: String,
        display_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_start_operation() {
            window.push_notification(
                Notification::warning("已有任务或内容操作正在运行，请等待完成或先停止它。")
                    .title("无法安装内容"),
                cx,
            );
            return;
        }
        window.push_notification(
            Notification::info(format!("正在安装 {display_name}。")).title("已开始下载"),
            cx,
        );
        self.spawn_command(Command::ContentInstall(request), instance_id, window, cx);
    }

    fn start_launch(&mut self, version_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.start_launch_with_server(version_id, None, window, cx);
    }

    fn start_launch_with_server(
        &mut self,
        version_id: String,
        server: Option<ServerAddress>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.launch_resolving {
            return;
        }
        if !self.can_start_operation() {
            window.push_notification(
                Notification::warning("已有任务或内容操作正在运行，请等待完成或先停止它。")
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
                    server,
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
                                if !view.can_start_operation() {
                                    window.push_notification(
                                        Notification::warning(
                                            "已有任务或内容操作正在运行，请等待完成或先停止它。",
                                        )
                                        .title("无法启动游戏"),
                                        cx,
                                    );
                                } else {
                                    view.spawn_launch(
                                        resolved.launch,
                                        version_id,
                                        server,
                                        window,
                                        cx,
                                    );
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
        server: Option<ServerAddress>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_start_operation() {
            window.push_notification(
                Notification::warning("已有任务或内容操作正在运行，请等待完成或先停止它。")
                    .title("无法启动游戏"),
                cx,
            );
            return;
        }
        let selected = self
            .installed_versions
            .iter()
            .find(|version| version.id.as_str() == version_id);
        let java_executable = selected.and_then(|version| version.java.clone());
        let max_memory_mb = selected.and_then(|version| version.max_memory_mb);
        let resolution =
            selected.and_then(|version| version.window_width.zip(version.window_height));
        let minecraft_version = selected
            .map(|version| version.minecraft_version.clone())
            .unwrap_or_else(|| version_id.clone());
        let request = GameLaunchRequest {
            install: VanillaInstallRequest {
                minecraft_root: self.config.minecraft_root.clone(),
                version_id: version_id.clone(),
                platform: self.services.platform.clone(),
                source: VersionSource::Existing,
            },
            account,
            java_executable,
            max_memory_mb,
            additional_jvm_arguments: Vec::new(),
            additional_game_arguments: Vec::new(),
            minecraft_version,
            server,
            resolution,
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
                        Command::CreateInstance(request) => {
                            command_manager.start_create_instance(request).await
                        }
                        Command::ContentInstall(request) => {
                            command_manager
                                .start_content_install_with_dependencies(request)
                                .await
                        }
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

    fn hide_for_game(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        if self.launcher_hidden {
            return;
        }
        self.launcher_hidden = true;
        #[cfg(target_os = "macos")]
        _cx.hide();
        #[cfg(not(target_os = "macos"))]
        _window.minimize_window();
    }

    fn restore_launcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.launcher_hidden {
            return;
        }
        self.launcher_hidden = false;
        #[cfg(target_os = "macos")]
        restore_macos_windows();
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
            TaskStatus::Completed if snapshot.kind == TaskKind::InstanceCreate => {
                let loader_source = self.create_copy_source.take();
                let is_loader_instance = loader_source.is_some();
                let (instance_id, name) = match snapshot.subject {
                    Some(TaskSubject::Instance { id, name }) => (Some(id), name),
                    _ => (None, "实例".to_owned()),
                };
                self.reload_versions(window, cx);
                if let Some(instance_id) = instance_id {
                    self.config.selected_version = Some(instance_id.clone());
                    self.selected_instance = Some(instance_id);
                    self.page = if is_loader_instance {
                        Page::Versions
                    } else {
                        Page::Instance
                    };
                    self.save_config();
                    if !is_loader_instance {
                        self.load_instance_content(window, cx);
                    }
                }
                if is_loader_instance {
                    window.push_notification(
                        Notification::success(format!(
                            "加载器实例 {name} 已创建，并复制了原实例内容。"
                        ))
                        .title("加载器安装完成"),
                        cx,
                    );
                } else {
                    window.push_notification(
                        Notification::success(format!("实例 {name} 已创建。"))
                            .title("实例创建完成"),
                        cx,
                    );
                }
            }
            TaskStatus::Completed if snapshot.kind == TaskKind::ContentInstall => {
                let (instance_id, name) = match snapshot.subject {
                    Some(TaskSubject::Content {
                        instance_id, name, ..
                    }) => (Some(instance_id), name),
                    _ => (None, "内容".to_owned()),
                };
                if self.page == Page::Instance
                    && self.selected_instance.as_deref() == instance_id.as_deref()
                {
                    self.load_instance_content(window, cx);
                }
                window.push_notification(
                    Notification::success(format!("{name} 已安装到实例。")).title("内容安装完成"),
                    cx,
                );
            }
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

            let mut list = v_flex().w_full().gap_1().max_h_80().overflow_y_scrollbar();
            for account in accounts.iter() {
                let account_id = account.uuid();
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
                        .id(format!("account-row-{account_id}"))
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
                                    this.select_account_key(&row_username, kind, cx);
                                    window.close_dialog(cx);
                                    cx.notify();
                                });
                            }
                        })
                        .child(account_avatar(
                            Some(account),
                            head,
                            layout::AVATAR_SMALL,
                            cx,
                        ))
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
                            Button::new(format!("account-delete-{account_id}"))
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
            dialog
                .title("账户")
                .width(layout::rem_px(cx, layout::DIALOG_MEDIUM))
                .child(list)
                .footer(
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
                .width(layout::rem_px(cx, layout::DIALOG_MEDIUM))
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
        self.refresh_account_heads(window, cx);
        if self.page == Page::Skin {
            self.ensure_skin_page(cx);
        }
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
                .width(layout::rem_px(cx, layout::DIALOG_SMALL))
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
        self.skin_service.update(cx, |service, cx| {
            service.remove_account(account.uuid(), cx);
        });
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
        if self.page == Page::Skin {
            self.skin_editor.draft = None;
            self.skin_apply = ApplyState::Idle;
            self.ensure_skin_page(cx);
        }
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
                                .child(version_tile(IconName::Download, cx.theme().primary, cx))
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
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("各实例的 mods、资源包、光影和存档互不影响。"),
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
        let isolated = self
            .installed_versions
            .iter()
            .find(|version| version.id.as_str() == version_id)
            .is_some_and(InstalledVersion::is_isolated);
        let description = if isolated {
            "将删除整个实例目录，包括 mods、资源包、光影和存档；共享 libraries 和 assets 不受影响。"
        } else {
            "将删除整个版本目录；共享 libraries 和 assets 不受影响。"
        };
        let view = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .confirm()
                .title(format!("删除实例 {version_id}？"))
                .description(description)
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
        if !self.can_start_operation() {
            window.push_notification(
                Notification::warning("请先等待当前任务或内容操作结束。").title("无法删除实例"),
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

    fn open_instance_folder(
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
        let directory = version.game_dir.clone();
        let _ = std::fs::create_dir_all(&directory);
        if let Err(error) = open_path(&directory) {
            window.push_notification(
                Notification::error(format!("无法打开实例目录: {error}")).title("打开失败"),
                cx,
            );
        }
    }

    fn open_game_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.selected_version().map_or_else(
            || self.config.minecraft_root.clone(),
            |version| version.game_dir.clone(),
        );
        let _ = std::fs::create_dir_all(&root);
        if let Err(error) = open_path(&root) {
            window.push_notification(
                Notification::error(format!("无法打开游戏目录: {error}")).title("打开失败"),
                cx,
            );
        }
    }

    fn confirm_enable_isolation(
        &mut self,
        version_id: String,
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
        let name = version.display_name().to_owned();
        let view = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .confirm()
                .title(format!("为 {name} 启用版本隔离？"))
                .description(
                    "游戏将使用实例目录作为工作目录。全局 mods、资源包和光影会复制到该实例，不覆盖已有文件，原全局内容保留。",
                )
                .ok_text("启用隔离")
                .on_ok({
                    let version_id = version_id.clone();
                    let view = view.clone();
                    move |_, window, cx| {
                        let Some(view) = view.upgrade() else {
                            return true;
                        };
                        view.update(cx, |this, cx| {
                            this.enable_isolation(version_id.clone(), window, cx)
                        });
                        true
                    }
                })
        });
    }

    fn enable_isolation(
        &mut self,
        version_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_start_operation() {
            window.push_notification(
                Notification::warning("请先等待当前任务或内容操作结束。").title("无法启用隔离"),
                cx,
            );
            return;
        }
        let display = version_id.clone();
        let root = self.config.minecraft_root.clone();
        let services = self.services.clone();
        self.log_state.begin_command();
        self._command_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = services
                .runtime
                .spawn(async move { migrate_instance_to_isolated(root, &version_id, true).await })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.log_state.finish_command();
                match result {
                    Ok(Ok(())) => {
                        view.reload_versions(window, cx);
                        view.save_config();
                        window.push_notification(
                            Notification::success(format!(
                                "已为 {display} 启用版本隔离，全局内容已复制到实例。"
                            ))
                            .title("版本隔离已启用"),
                            cx,
                        );
                    }
                    Ok(Err(error)) => {
                        window.push_notification(
                            Notification::error(error.to_string()).title("启用隔离失败"),
                            cx,
                        );
                    }
                    Err(error) => {
                        window.push_notification(
                            Notification::error(format!("后台任务失败: {error}"))
                                .title("启用隔离失败"),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn selected_instance_version(&self) -> Option<&InstalledVersion> {
        let selected = self.selected_instance.as_deref()?;
        self.installed_versions
            .iter()
            .find(|version| version.id.as_str() == selected)
    }

    fn open_instance_detail(
        &mut self,
        version_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.content_action_pending {
            window.push_notification(
                Notification::warning("已有内容操作正在进行，请稍候。").title("无法打开实例"),
                cx,
            );
            return;
        }
        if !self
            .installed_versions
            .iter()
            .any(|version| version.id.as_str() == version_id)
        {
            window.push_notification(
                Notification::error("实例不存在或已被删除。").title("无法打开实例"),
                cx,
            );
            return;
        }
        self.selected_instance = Some(version_id);
        self.page = Page::Instance;
        self.content_tab = ContentTab::Mods;
        self.content_library = None;
        self.content_error = None;
        self.content_query.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        self.load_instance_content(window, cx);
    }

    fn open_rename_instance_dialog(
        &mut self,
        version_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(version) = self
            .installed_versions
            .iter()
            .find(|version| version.id.as_str() == version_id)
            .cloned()
        else {
            return;
        };
        let initial = if version.name.trim() == version.id.as_str() {
            String::new()
        } else {
            version.name.clone()
        };
        self.instance_name_input.update(cx, |state, cx| {
            state.set_value(initial, window, cx);
        });

        let input = self.instance_name_input.clone();
        let view = cx.entity().downgrade();
        let first_focus = Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if first_focus.replace(false) {
                input.read(cx).focus_handle(cx).focus(window, cx);
            }
            let save_view = view.clone();
            let save_input = input.clone();
            let save_id = version_id.clone();
            dialog
                .title("重命名实例")
                .width(layout::rem_px(cx, layout::DIALOG_SMALL))
                .child(
                    v_flex().gap_3().child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().font_medium().child("显示名称"))
                            .child(div().p_1().child(Input::new(&input).w_full()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("留空时显示实例 ID；不会重命名目录。"),
                            ),
                    ),
                )
                .footer(
                    DialogFooter::new()
                        .child(DialogClose::new().trigger(|button| button.label("取消").outline()))
                        .child(
                            DialogAction::new()
                                .child(Button::new("save-instance-name").primary().label("保存")),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    let Some(view) = save_view.upgrade() else {
                        return true;
                    };
                    view.update(cx, |this, cx| {
                        this.save_instance_name(
                            save_id.clone(),
                            save_input.read(cx).value().to_string(),
                            window,
                            cx,
                        )
                    })
                })
        });
    }

    fn save_instance_name(
        &mut self,
        version_id: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(version) = self
            .installed_versions
            .iter()
            .find(|version| version.id.as_str() == version_id)
            .cloned()
        else {
            window.push_notification(
                Notification::error("实例不存在或已被删除。").title("无法重命名实例"),
                cx,
            );
            return false;
        };
        let layout = version.instance_layout(self.config.minecraft_root.clone());
        match rename_instance(&layout, &name) {
            Ok(()) => {
                self.reload_versions(window, cx);
                window.push_notification(
                    Notification::info(if name.trim().is_empty() {
                        version.id.as_str().to_owned()
                    } else {
                        name.trim().to_owned()
                    })
                    .title("实例已重命名"),
                    cx,
                );
                true
            }
            Err(error) => {
                window.push_notification(
                    Notification::error(error.to_string()).title("无法重命名实例"),
                    cx,
                );
                false
            }
        }
    }

    fn open_launch_settings_dialog(
        &mut self,
        version_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(version) = self
            .installed_versions
            .iter()
            .find(|version| version.id.as_str() == version_id)
            .cloned()
        else {
            return;
        };
        let memory = version
            .max_memory_mb
            .map(|value| value.to_string())
            .unwrap_or_default();
        let width = version
            .window_width
            .map(|value| value.to_string())
            .unwrap_or_default();
        let height = version
            .window_height
            .map(|value| value.to_string())
            .unwrap_or_default();
        self.launch_memory_input.update(cx, |state, cx| {
            state.set_value(memory, window, cx);
        });
        self.launch_width_input.update(cx, |state, cx| {
            state.set_value(width, window, cx);
        });
        self.launch_height_input.update(cx, |state, cx| {
            state.set_value(height, window, cx);
        });

        let memory_input = self.launch_memory_input.clone();
        let width_input = self.launch_width_input.clone();
        let height_input = self.launch_height_input.clone();
        let view = cx.entity().downgrade();
        let first_focus = Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if first_focus.replace(false) {
                memory_input.read(cx).focus_handle(cx).focus(window, cx);
            }
            let save_view = view.clone();
            let save_memory = memory_input.clone();
            let save_width = width_input.clone();
            let save_height = height_input.clone();
            let save_id = version_id.clone();
            dialog
                .title("启动设置")
                .width(layout::rem_px(cx, layout::DIALOG_MEDIUM))
                .child(
                    v_flex()
                        .gap_4()
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_sm().font_medium().child("最大内存 (MiB)"))
                                .child(div().child(Input::new(&memory_input).w_full()))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("留空使用 JVM 默认值；允许 256–262144。"),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_sm().font_medium().child("窗口尺寸"))
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            div().flex_1().child(Input::new(&width_input).w_full()),
                                        )
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(cx.theme().muted_foreground)
                                                .child("×"),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .child(Input::new(&height_input).w_full()),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(
                                            "宽高需同时填写，允许 320–16384；留空使用游戏默认。",
                                        ),
                                ),
                        ),
                )
                .footer(
                    DialogFooter::new()
                        .child(DialogClose::new().trigger(|button| button.label("取消").outline()))
                        .child(
                            DialogAction::new()
                                .child(Button::new("save-launch-settings").primary().label("保存")),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    let Some(view) = save_view.upgrade() else {
                        return true;
                    };
                    view.update(cx, |this, cx| {
                        this.save_launch_settings(
                            save_id.clone(),
                            save_memory.read(cx).value().to_string(),
                            save_width.read(cx).value().to_string(),
                            save_height.read(cx).value().to_string(),
                            window,
                            cx,
                        )
                    })
                })
        });
    }

    fn save_launch_settings(
        &mut self,
        version_id: String,
        memory: String,
        width: String,
        height: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let parse = |value: &str| -> Result<Option<u32>, ()> {
            let value = value.trim();
            if value.is_empty() {
                Ok(None)
            } else {
                value.parse::<u32>().map(Some).map_err(|_| ())
            }
        };
        let (Ok(memory), Ok(width), Ok(height)) = (parse(&memory), parse(&width), parse(&height))
        else {
            window.push_notification(
                Notification::error("内存和窗口尺寸必须是整数。").title("无法保存启动设置"),
                cx,
            );
            return false;
        };
        let Some(version) = self
            .installed_versions
            .iter()
            .find(|version| version.id.as_str() == version_id)
            .cloned()
        else {
            window.push_notification(
                Notification::error("实例不存在或已被删除。").title("无法保存启动设置"),
                cx,
            );
            return false;
        };
        let layout = version.instance_layout(self.config.minecraft_root.clone());
        match set_instance_launch_settings(&layout, memory, width, height) {
            Ok(()) => {
                self.reload_versions(window, cx);
                window.push_notification(
                    Notification::info("内存和窗口设置已更新。").title("启动设置已保存"),
                    cx,
                );
                true
            }
            Err(error) => {
                window.push_notification(
                    Notification::error(error.to_string()).title("无法保存启动设置"),
                    cx,
                );
                false
            }
        }
    }

    fn open_java_dialog(
        &mut self,
        version_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .installed_versions
            .iter()
            .all(|version| version.id.as_str() != version_id)
        {
            return;
        }
        let initial_path = self
            .installed_versions
            .iter()
            .find(|version| version.id.as_str() == version_id)
            .and_then(|version| version.java.as_ref())
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        self.java_path_input.update(cx, |state, cx| {
            state.set_value(initial_path, window, cx);
        });
        self.java_dialog_target = Some(version_id);
        self.java_dialog_runtimes.clear();
        self.java_dialog_error = None;
        self.java_custom_probing = false;
        self.spawn_java_discovery(window, cx);

        let java_input = self.java_path_input.clone();
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let Some(view) = view.upgrade() else {
                return dialog.title("选择 Java");
            };
            let this = view.read(cx);
            let loading = this.java_dialog_loading;
            let error = this.java_dialog_error.clone();
            let runtimes = this.java_dialog_runtimes.clone();
            let target = this.java_dialog_target.clone().unwrap_or_default();
            let custom_probing = this.java_custom_probing;
            let current = this
                .installed_versions
                .iter()
                .find(|version| version.id.as_str() == target)
                .and_then(|version| version.java.clone());

            let mut list = v_flex().gap_1();
            let auto_view = view.downgrade();
            let auto_target = target.clone();
            let auto_selected = current.is_none();
            list = list.child(
                h_flex()
                    .id("java-runtime-auto")
                    .w_full()
                    .p_2()
                    .gap_3()
                    .rounded(cx.theme().radius)
                    .when(auto_selected, |this| {
                        this.bg(cx.theme().primary.opacity(0.10))
                    })
                    .hover(|style| style.bg(cx.theme().accent.opacity(0.45)))
                    .on_click(move |_, window, cx| {
                        if let Some(view) = auto_view.upgrade() {
                            view.update(cx, |this, cx| {
                                this.apply_instance_java(auto_target.clone(), None, window, cx);
                            });
                            window.close_dialog(cx);
                        }
                    })
                    .child(
                        div()
                            .size_8()
                            .rounded(cx.theme().radius)
                            .bg(cx.theme().primary.opacity(0.12))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                Icon::new(IconName::Sparkles)
                                    .with_size(Size::Small)
                                    .text_color(cx.theme().primary),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(div().text_sm().font_medium().child("自动选择"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("按游戏版本选择兼容 runtime，缺失时自动获取"),
                            ),
                    )
                    .when(auto_selected, |this| {
                        this.child(
                            Icon::new(IconName::CircleCheck)
                                .with_size(Size::Small)
                                .text_color(cx.theme().primary),
                        )
                    }),
            );

            for runtime in runtimes.into_iter() {
                let selected = current.as_ref() == Some(&runtime.executable);
                let executable = runtime.executable.clone();
                let runtime_key = runtime.executable.display().to_string();
                let row_view = view.downgrade();
                let row_target = target.clone();
                let source = java_source_label(runtime.source);
                list = list.child(
                    h_flex()
                        .id(format!("java-runtime-{runtime_key}"))
                        .w_full()
                        .p_2()
                        .gap_3()
                        .rounded(cx.theme().radius)
                        .when(selected, |this| this.bg(cx.theme().primary.opacity(0.10)))
                        .hover(|style| style.bg(cx.theme().accent.opacity(0.45)))
                        .on_click(move |_, window, cx| {
                            if let Some(view) = row_view.upgrade() {
                                view.update(cx, |this, cx| {
                                    this.apply_instance_java(
                                        row_target.clone(),
                                        Some(executable.clone()),
                                        window,
                                        cx,
                                    );
                                });
                                window.close_dialog(cx);
                            }
                        })
                        .child(
                            div()
                                .size_8()
                                .rounded(cx.theme().radius)
                                .bg(cx.theme().primary.opacity(0.12))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    Icon::new(IconName::Package)
                                        .with_size(Size::Small)
                                        .text_color(cx.theme().primary),
                                ),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_0p5()
                                .child(div().text_sm().font_medium().truncate().child(format!(
                                    "Java {} · {}",
                                    runtime.major_version, runtime.version
                                )))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .truncate()
                                        .child(format!(
                                            "{source} · {}",
                                            runtime.executable.display()
                                        )),
                                ),
                        )
                        .when(selected, |this| {
                            this.child(
                                Icon::new(IconName::CircleCheck)
                                    .with_size(Size::Small)
                                    .text_color(cx.theme().primary),
                            )
                        }),
                );
            }

            let custom_view = view.downgrade();
            let custom_target = target.clone();
            let custom_input = java_input.clone();
            let body = v_flex()
                .gap_3()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("仅影响当前实例。自动选择会跟随游戏版本和 Java 自动获取设置。"),
                )
                .when(loading, |this| {
                    this.child(
                        h_flex()
                            .gap_2()
                            .py_4()
                            .child(
                                Spinner::new()
                                    .with_size(Size::Small)
                                    .color(cx.theme().primary),
                            )
                            .child(div().text_sm().child("正在探测本机 Java…")),
                    )
                })
                .when_some(error, |this, error: String| {
                    this.child(div().text_sm().text_color(cx.theme().danger).child(error))
                })
                .child(list)
                .child(
                    v_flex()
                        .gap_2()
                        .pt_2()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .child(div().text_sm().font_medium().child("自定义可执行文件"))
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(Input::new(&java_input).w_full()),
                                )
                                .child(
                                    Button::new("java-custom-use")
                                        .outline()
                                        .label(if custom_probing {
                                            "检测中…"
                                        } else {
                                            "使用此路径"
                                        })
                                        .disabled(custom_probing || loading)
                                        .on_click(move |_, window, cx| {
                                            let path =
                                                custom_input.read(cx).value().trim().to_owned();
                                            if path.is_empty() {
                                                return;
                                            }
                                            if let Some(view) = custom_view.upgrade() {
                                                view.update(cx, |this, cx| {
                                                    this.probe_and_apply_java(
                                                        custom_target.clone(),
                                                        PathBuf::from(path),
                                                        window,
                                                        cx,
                                                    );
                                                });
                                            }
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("必须指向 java 或 java.exe；启动时会重新校验版本与平台。"),
                        ),
                );

            let retry_view = view.downgrade();
            dialog
                .title("选择 Java")
                .width(layout::rem_px(cx, layout::DIALOG_LARGE))
                .child(body)
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("java-dialog-close")
                                .ghost()
                                .label("关闭")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("java-dialog-retry")
                                .outline()
                                .label("重新检测")
                                .disabled(loading)
                                .on_click(move |_, window, cx| {
                                    if let Some(view) = retry_view.upgrade() {
                                        view.update(cx, |this, cx| {
                                            this.spawn_java_discovery(window, cx)
                                        });
                                    }
                                }),
                        ),
                )
        });
    }

    fn spawn_java_discovery(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.java_dialog_loading = true;
        self.java_dialog_error = None;
        self.java_dialog_runtimes.clear();
        cx.notify();
        let java = self.services.java.clone();
        let runtime = self.services.runtime.handle().clone();
        let view = cx.entity().downgrade();
        self._java_task = Some(cx.spawn_in(window, async move |_, cx| {
            let discovery = runtime.spawn(async move { java.discover().await }).await;
            let _ = view.update_in(cx, |this, _, cx| {
                this.java_dialog_loading = false;
                match discovery {
                    Ok(discovery) => {
                        let failure_count = discovery.failures.len();
                        this.java_dialog_runtimes = discovery.runtimes;
                        if this.java_dialog_runtimes.is_empty() {
                            this.java_dialog_error = Some(if failure_count == 0 {
                                "没有找到 Java。请检查 JAVA_HOME、PATH 或 RMCL 管理的 runtime。"
                                    .to_owned()
                            } else {
                                format!("没有可用 Java；{failure_count} 个候选探测失败。")
                            });
                        }
                    }
                    Err(error) => {
                        this.java_dialog_error = Some(format!("Java 探测任务失败：{error}"));
                    }
                }
                cx.notify();
            });
        }));
    }

    fn probe_and_apply_java(
        &mut self,
        version_id: String,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.java_custom_probing = true;
        cx.notify();
        let java = self.services.java.clone();
        let runtime = self.services.runtime.handle().clone();
        let view = cx.entity().downgrade();
        self._java_task = Some(cx.spawn_in(window, async move |_, cx| {
            let result = runtime
                .spawn(async move { java.probe(path).await.map_err(|error| error.to_string()) })
                .await;
            let result = result.unwrap_or_else(|error| Err(format!("Java 探测任务失败：{error}")));
            let _ = view.update_in(cx, |this, window, cx| {
                this.java_custom_probing = false;
                match result {
                    Ok(runtime) => {
                        if this.apply_instance_java(
                            version_id,
                            Some(runtime.executable),
                            window,
                            cx,
                        ) {
                            window.close_dialog(cx);
                        }
                    }
                    Err(error) => {
                        window.push_notification(
                            Notification::error(error).title("无法使用该 Java"),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
    }

    fn apply_instance_java(
        &mut self,
        version_id: String,
        java: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(version) = self
            .installed_versions
            .iter()
            .find(|version| version.id.as_str() == version_id)
            .cloned()
        else {
            window.push_notification(
                Notification::error("实例不存在或已被删除。").title("无法保存 Java 设置"),
                cx,
            );
            return false;
        };
        let layout = version.instance_layout(self.config.minecraft_root.clone());
        match set_instance_java(&layout, java.clone()) {
            Ok(()) => {
                self.java_dialog_target = None;
                self.reload_versions(window, cx);
                let label =
                    java.map_or_else(|| "自动选择".to_owned(), |path| path.display().to_string());
                window.push_notification(Notification::info(label).title("实例 Java 已更新"), cx);
                true
            }
            Err(error) => {
                window.push_notification(
                    Notification::error(error.to_string()).title("无法保存 Java 设置"),
                    cx,
                );
                false
            }
        }
    }

    fn open_create_loader(
        &mut self,
        source_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(version) = self
            .installed_versions
            .iter()
            .find(|version| version.id.as_str() == source_id)
            .cloned()
        else {
            window.push_notification(
                Notification::error("实例不存在或已被移动。").title("无法添加加载器"),
                cx,
            );
            return;
        };
        if let Some(loader) = version.loader.as_ref() {
            window.push_notification(
                Notification::warning(format!(
                    "当前实例已安装 {} {}。",
                    loader.kind.label(),
                    loader.version
                ))
                .title("无法添加加载器"),
                cx,
            );
            return;
        }
        if !version.manifest_present {
            window.push_notification(
                Notification::warning("旧版实例缺少 RMCL 配置，暂不支持添加加载器。")
                    .title("无法添加加载器"),
                cx,
            );
            return;
        }
        if let Some(error) = version.manifest_error.as_ref() {
            window.push_notification(
                Notification::error(error.clone()).title("无法添加加载器"),
                cx,
            );
            return;
        }
        if !self.can_start_operation() {
            window.push_notification(
                Notification::warning("已有任务或内容操作正在运行，请等待完成或先停止它。")
                    .title("无法添加加载器"),
                cx,
            );
            return;
        }
        if let Some(cancellation) = self.create_loader_cancel.take() {
            cancellation.cancel();
        }
        self.create_copy_source = Some(source_id);
        self.create_loader = Some(LoaderKind::Fabric);
        self.create_loader_versions.clear();
        self.create_loader_version = None;
        self.create_loader_loading = false;
        self.create_loader_error = None;
        self.create_base_input.update(cx, |state, cx| {
            state.set_value(version.minecraft_version.clone(), window, cx);
        });
        self.create_name_input.update(cx, |state, cx| {
            state.set_value(
                format!("{} {}", LoaderKind::Fabric.label(), version.display_name()),
                window,
                cx,
            );
        });
        self.create_id_input.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        self.navigate(Page::CreateInstance, window, cx);
        self.fetch_loader_versions(window, cx);
    }

    /// Rebuilds the create-instance version picker from the cached Mojang
    /// manifest. When the user has not chosen a base version yet, the latest
    /// release becomes the default.
    fn populate_create_version_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(manifest) = self.catalog.manifest.clone() else {
            return;
        };
        let current = self.create_base_input.read(cx).value().trim().to_owned();
        let default = if current.is_empty()
            && self.page == Page::CreateInstance
            && self.create_copy_source.is_none()
        {
            manifest
                .latest
                .as_ref()
                .and_then(|latest| latest.release.clone())
                .or_else(|| manifest.versions.first().map(|version| version.id.clone()))
        } else {
            None
        };
        let base = default.clone().unwrap_or(current);
        let choices = manifest
            .versions
            .iter()
            .cloned()
            .map(CreateVersionChoice::new)
            .collect::<Vec<_>>();
        self.create_version_select.update(cx, |state, cx| {
            state.set_items(SearchableVec::new(choices), window, cx);
            if base.is_empty() {
                state.set_selected_values(&[], window, cx);
            } else {
                state.set_selected_values(std::slice::from_ref(&base), window, cx);
            }
        });
        if let Some(default) = default {
            self.create_base_input.update(cx, |state, cx| {
                state.set_value(default, window, cx);
            });
        }
    }

    /// Mirrors the current base version into the picker without changing the
    /// value stored in the form.
    fn sync_create_version_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let base = self.create_base_input.read(cx).value().trim().to_owned();
        self.create_version_select.update(cx, |state, cx| {
            if base.is_empty() {
                state.set_selected_values(&[], window, cx);
            } else {
                state.set_selected_values(std::slice::from_ref(&base), window, cx);
            }
        });
    }

    fn open_create_instance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(cancellation) = self.create_loader_cancel.take() {
            cancellation.cancel();
        }
        self.create_copy_source = None;
        self.page = Page::CreateInstance;
        self.create_loader = None;
        self.create_loader_versions.clear();
        self.create_loader_version = None;
        self.create_loader_loading = false;
        self.create_loader_error = None;
        let base = self
            .selected_instance_version()
            .map(|version| version.minecraft_version.clone())
            .or_else(|| self.latest_release().map(str::to_owned))
            .unwrap_or_default();
        self.create_base_input.update(cx, |state, cx| {
            state.set_value(base, window, cx);
        });
        self.sync_create_version_selection(window, cx);
        if self.catalog.manifest.is_none() && !self.catalog.loading {
            self.load_catalog(false, window, cx);
        }
        self.create_name_input.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        self.create_id_input.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        cx.notify();
    }

    fn select_create_loader(
        &mut self,
        loader: Option<LoaderKind>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.create_loader == loader {
            return;
        }
        self.create_loader = loader;
        if self.create_copy_source.is_some() {
            let source_name = self
                .create_copy_source
                .as_ref()
                .and_then(|id| {
                    self.installed_versions
                        .iter()
                        .find(|version| version.id.as_str() == id.as_str())
                })
                .map(|version| version.display_name().to_owned())
                .unwrap_or_else(|| "实例".to_owned());
            let name = loader.map_or(source_name.clone(), |kind| {
                format!("{} {}", kind.label(), source_name)
            });
            self.create_name_input.update(cx, |state, cx| {
                state.set_value(name, window, cx);
            });
        }
        self.create_loader_versions.clear();
        self.create_loader_version = None;
        self.create_loader_error = None;
        if let Some(cancellation) = self.create_loader_cancel.take() {
            cancellation.cancel();
        }
        if loader.is_some() {
            self.fetch_loader_versions(window, cx);
        }
        cx.notify();
    }

    fn fetch_loader_versions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(kind) = self.create_loader else {
            return;
        };
        let base = self.create_base_input.read(cx).value().trim().to_owned();
        if base.is_empty() {
            self.create_loader_error = Some("请先选择 Minecraft 版本。".to_owned());
            cx.notify();
            return;
        }
        if let Some(cancellation) = self.create_loader_cancel.take() {
            cancellation.cancel();
        }
        let cancellation = CancellationToken::new();
        self.create_loader_cancel = Some(cancellation.clone());
        self.create_loader_request = self.create_loader_request.wrapping_add(1);
        let request = self.create_loader_request;
        self.create_loader_loading = true;
        self.create_loader_error = None;
        let client = self.services.loaders.clone();
        let runtime = self.services.runtime.handle().clone();
        self._create_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = runtime
                .spawn(async move { client.versions(kind, &base, &cancellation).await })
                .await;
            let _ = this.update_in(cx, |view, _, cx| {
                if view.create_loader_request != request {
                    return;
                }
                view.create_loader_loading = false;
                match result {
                    Ok(Ok(versions)) => {
                        view.create_loader_version = versions
                            .iter()
                            .find(|version| version.stable)
                            .or_else(|| versions.first())
                            .cloned();
                        view.create_loader_versions = versions;
                        view.create_loader_error = None;
                    }
                    Ok(Err(error)) => {
                        if !matches!(error, AppError::Cancelled) {
                            view.create_loader_error = Some(error.to_string());
                        }
                    }
                    Err(error) => {
                        view.create_loader_error = Some(format!("后台任务失败: {error}"));
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn generated_instance_id(&self, cx: &App) -> String {
        let base = self.create_base_input.read(cx).value().trim().to_owned();
        if base.is_empty() {
            return String::new();
        }
        match self.create_loader {
            None => {
                if self
                    .installed_versions
                    .iter()
                    .any(|version| version.id.as_str() == base)
                {
                    format!("{base}-custom")
                } else {
                    base
                }
            }
            Some(LoaderKind::Fabric) => {
                let loader = self
                    .create_loader_version
                    .as_ref()
                    .map(|version| version.version.as_str())
                    .unwrap_or("latest");
                format!("fabric-loader-{loader}-{base}")
            }
            Some(LoaderKind::Quilt) => {
                let loader = self
                    .create_loader_version
                    .as_ref()
                    .map(|version| version.version.as_str())
                    .unwrap_or("latest");
                format!("quilt-loader-{loader}-{base}")
            }
            Some(LoaderKind::Forge) => {
                let loader = self
                    .create_loader_version
                    .as_ref()
                    .map(|version| version.version.as_str())
                    .unwrap_or("latest");
                format!("forge-{loader}-{base}")
            }
            Some(LoaderKind::NeoForge) => {
                let loader = self
                    .create_loader_version
                    .as_ref()
                    .map(|version| version.version.as_str())
                    .unwrap_or("latest");
                format!("neoforge-{loader}-{base}")
            }
        }
    }

    fn create_instance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_start_operation() {
            window.push_notification(
                Notification::warning("已有任务或内容操作正在运行，请等待完成或先停止它。")
                    .title("无法创建实例"),
                cx,
            );
            return;
        }
        if self.create_copy_source.is_some() && self.create_loader.is_none() {
            window.push_notification(
                Notification::error("请选择一个 Mod 加载器。").title("无法添加加载器"),
                cx,
            );
            return;
        }
        let base = self.create_base_input.read(cx).value().trim().to_owned();
        if base.is_empty() {
            window.push_notification(
                Notification::error("请选择 Minecraft 版本。").title("无法创建实例"),
                cx,
            );
            return;
        }
        let id = {
            let entered = self.create_id_input.read(cx).value().trim().to_owned();
            if entered.is_empty() {
                self.generated_instance_id(cx)
            } else {
                entered
            }
        };
        if id.is_empty() {
            window.push_notification(
                Notification::error("实例 ID 不能为空。").title("无法创建实例"),
                cx,
            );
            return;
        }
        let name = self.create_name_input.read(cx).value().trim().to_owned();
        let loader = match self.create_loader {
            Some(kind) => {
                let Some(version) = self.create_loader_version.clone() else {
                    window.push_notification(
                        Notification::error("请先获取并选择加载器版本。").title("无法创建实例"),
                        cx,
                    );
                    return;
                };
                Some(LoaderSelection {
                    kind,
                    version: version.version,
                    installer_version: version.installer_version,
                })
            }
            None => None,
        };
        let request = CreateInstanceRequest {
            minecraft_root: self.config.minecraft_root.clone(),
            instance_id: id.clone(),
            name,
            base_version_id: base,
            platform: self.services.platform.clone(),
            loader,
            copy_content_from: self.create_copy_source.clone(),
        };
        self.start_create_instance_task(request, id, window, cx);
    }

    fn start_create_instance_task(
        &mut self,
        request: CreateInstanceRequest,
        instance_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.push_notification(
            Notification::info(format!("正在创建实例 {instance_id}。")).title("已开始创建"),
            cx,
        );
        self.spawn_command(Command::CreateInstance(request), instance_id, window, cx);
    }

    fn open_content_browser(
        &mut self,
        kind: ContentKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(version) = self.selected_instance_version().cloned() else {
            window.push_notification(
                Notification::error("实例不存在或已被删除。").title("无法打开在线内容"),
                cx,
            );
            return;
        };
        self.cancel_content_browser_request();
        self.selected_instance = Some(version.id.as_str().to_owned());
        self.content_tab = match kind {
            ContentKind::Mod => ContentTab::Mods,
            ContentKind::ResourcePack => ContentTab::ResourcePacks,
            ContentKind::ShaderPack => ContentTab::ShaderPacks,
        };
        self.page = Page::ContentBrowser;
        self.content_browser = ContentBrowserState {
            kind: Some(kind),
            ..ContentBrowserState::default()
        };
        self.remote_sort = RemoteSort::Relevance;
        self.remote_query.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        self.load_remote_search(true, window, cx);
    }

    fn back_to_instance_detail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_content_browser_request();
        self.page = Page::Instance;
        self.load_instance_content(window, cx);
    }

    fn cancel_content_browser_request(&mut self) {
        if let Some(cancellation) = self.content_browser_cancel.take() {
            cancellation.cancel();
        }
        self.content_browser_request = self.content_browser_request.wrapping_add(1);
    }

    fn load_remote_search(&mut self, reset: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !reset && self.content_browser.search_loading {
            return;
        }
        let Some(version) = self.selected_instance_version().cloned() else {
            return;
        };
        let Some(kind) = self.content_browser.kind else {
            return;
        };
        if let Some(cancellation) = self.content_browser_cancel.take() {
            cancellation.cancel();
        }
        let cancellation = CancellationToken::new();
        self.content_browser_cancel = Some(cancellation.clone());
        self.content_browser_request = self.content_browser_request.wrapping_add(1);
        let request = self.content_browser_request;
        let query = RemoteSearchQuery {
            source: ContentSource::Modrinth,
            kind,
            query: self.remote_query.read(cx).value().to_string(),
            game_version: Some(version.minecraft_version.clone()),
            loader: (kind == ContentKind::Mod)
                .then(|| {
                    version
                        .loader
                        .as_ref()
                        .map(|loader| loader.kind.as_str().to_owned())
                })
                .flatten(),
            offset: if reset {
                0
            } else {
                self.content_browser.projects.len()
            },
            limit: 20,
            sort: self.remote_sort,
        };
        self.content_browser.search_loading = true;
        self.content_browser.search_error = None;
        let client = self.services.modrinth.clone();
        let runtime = self.services.runtime.handle().clone();
        self._content_browser_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = runtime
                .spawn(async move { client.search(&query, &cancellation).await })
                .await;
            let _ = this.update_in(cx, |view, _, cx| {
                if view.content_browser_request != request {
                    return;
                }
                view.content_browser.search_loading = false;
                match result {
                    Ok(Ok(page)) => {
                        if reset {
                            view.content_browser.projects = page.projects;
                        } else {
                            view.content_browser.projects.extend(page.projects);
                        }
                        view.content_browser.total_hits = page.total_hits;
                        view.content_browser.search_error = None;
                    }
                    Ok(Err(error)) => {
                        if !matches!(error, rmcl_app::RemoteContentError::Cancelled) {
                            view.content_browser.search_error = Some(error.to_string());
                        }
                    }
                    Err(error) => {
                        view.content_browser.search_error = Some(format!("后台任务失败: {error}"));
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn open_remote_project(
        &mut self,
        project: RemoteProject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.content_browser.project = Some(project.clone());
        self.content_browser.versions.clear();
        self.content_browser.versions_error = None;
        self.load_remote_versions(project, window, cx);
    }

    fn load_remote_versions(
        &mut self,
        project: RemoteProject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(version) = self.selected_instance_version().cloned() else {
            return;
        };
        let game_version = version.minecraft_version.clone();
        let loader = project
            .version_loader_filter(version.loader.as_ref().map(|loader| loader.kind.as_str()));
        let project_id = project.id.clone();
        let recommended_kind = self.content_browser.kind;
        self.content_browser.recommended = None;
        self.content_browser.preflighting = false;
        if let Some(cancellation) = self.content_browser_cancel.take() {
            cancellation.cancel();
        }
        let cancellation = CancellationToken::new();
        self.content_browser_cancel = Some(cancellation.clone());
        self.content_browser_request = self.content_browser_request.wrapping_add(1);
        let request = self.content_browser_request;
        self.content_browser.versions_loading = true;
        self.content_browser.versions_error = None;
        let client = self.services.modrinth.clone();
        let runtime = self.services.runtime.handle().clone();
        self._content_browser_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = runtime
                .spawn(async move {
                    client
                        .project_versions(
                            &project_id,
                            Some(&game_version),
                            loader.as_deref(),
                            &cancellation,
                        )
                        .await
                })
                .await;
            let _ = this.update_in(cx, |view, _, cx| {
                if view.content_browser_request != request {
                    return;
                }
                view.content_browser.versions_loading = false;
                match result {
                    Ok(Ok(versions)) => {
                        view.content_browser.recommended = recommended_kind.and_then(|kind| {
                            recommended_version(&versions, kind).map(|version| version.id.clone())
                        });
                        view.content_browser.versions = versions;
                        view.content_browser.versions_error = None;
                    }
                    Ok(Err(error)) => {
                        if !matches!(error, rmcl_app::RemoteContentError::Cancelled) {
                            view.content_browser.versions_error = Some(error.to_string());
                        }
                    }
                    Err(error) => {
                        view.content_browser.versions_error =
                            Some(format!("后台任务失败: {error}"));
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn back_to_content_search(&mut self, cx: &mut Context<Self>) {
        self.cancel_content_browser_request();
        self.content_browser.project = None;
        self.content_browser.versions.clear();
        self.content_browser.versions_error = None;
        self.content_browser.versions_loading = false;
        cx.notify();
    }

    fn install_remote_version(
        &mut self,
        version: RemoteVersion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(kind) = self.content_browser.kind else {
            return;
        };
        let Some(file) = version.installable_file(kind) else {
            window.push_notification(
                Notification::error("该版本没有适用于当前内容类型的文件。").title("无法安装"),
                cx,
            );
            return;
        };
        if file.sha1.is_none() {
            window.push_notification(
                Notification::error("该文件没有 SHA-1 校验值，已拒绝安装。").title("无法安装"),
                cx,
            );
            return;
        }
        let Some(instance) = self.selected_instance_version().cloned() else {
            return;
        };
        let instance_layout = instance.instance_layout(self.config.minecraft_root.clone());
        let project_title = self
            .content_browser
            .project
            .as_ref()
            .map(|project| project.title.clone())
            .unwrap_or_else(|| kind.label().to_owned());
        let display_name = format!("{project_title} {}", version.version_number);
        let request = ContentGraphInstallRequest {
            instance: instance_layout,
            kind,
            version,
            display_name: display_name.clone(),
            conflict: ConflictPolicy::Rename,
            game_version: instance.minecraft_version.clone(),
            loader: instance
                .loader
                .as_ref()
                .map(|loader| loader.kind.as_str().to_owned()),
            skip_version_ids: Vec::new(),
        };
        if self.content_browser.preflighting {
            return;
        }
        self.content_browser.preflighting = true;
        cx.notify();
        let client = self.services.modrinth.clone();
        let metadata_cache = self.services.mod_metadata.clone();
        let runtime = self.services.runtime.handle().clone();
        let task_request = request.clone();
        let dialog_request = request.clone();
        let instance_id = instance.id.as_str().to_owned();
        self._content_browser_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = runtime
                .spawn(async move {
                    let scan_instance = task_request.instance.clone();
                    let installed =
                        tokio::task::spawn_blocking(move || scan_installed_mods(&scan_instance))
                            .await
                            .map_err(|error| AppError::BlockingTask(error.to_string()))?;
                    preflight_content_install(
                        &client,
                        &task_request,
                        &installed,
                        Some(&metadata_cache),
                        &CancellationToken::new(),
                    )
                    .await
                })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.content_browser.preflighting = false;
                if view.page != Page::ContentBrowser {
                    return;
                }
                match result {
                    Ok(Ok(preflight)) => view.open_content_preflight_dialog(
                        preflight,
                        dialog_request,
                        instance_id,
                        display_name,
                        window,
                        cx,
                    ),
                    Ok(Err(AppError::Cancelled)) => {}
                    Ok(Err(error)) => {
                        window.push_notification(
                            Notification::error(error.to_string()).title("无法检查安装"),
                            cx,
                        );
                    }
                    Err(error) => {
                        window.push_notification(
                            Notification::error(format!("后台任务失败: {error}"))
                                .title("无法检查安装"),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
    }

    fn open_content_preflight_dialog(
        &mut self,
        preflight: ContentInstallPreflight,
        mut request: ContentGraphInstallRequest,
        instance_id: String,
        display_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !preflight.incompatible.is_empty() {
            let names = preflight
                .incompatible
                .iter()
                .map(|item| item.title.as_str())
                .collect::<Vec<_>>()
                .join("、");
            window.push_notification(
                Notification::error(format!("检测到不兼容的已安装 Mod：{names}")).title("无法安装"),
                cx,
            );
            return;
        }
        request.skip_version_ids = preflight.skip_version_ids.clone();
        let view = cx.entity().downgrade();
        let dialog_preflight = preflight.clone();
        let dialog_request = request.clone();
        window.open_dialog(cx, move |dialog, _window, cx| {
            let save_view = view.clone();
            let save_request = dialog_request.clone();
            let save_instance = instance_id.clone();
            let save_name = display_name.clone();
            dialog
                .title(format!("安装 {display_name}"))
                .width(layout::rem_px(cx, layout::DIALOG_LARGE))
                .child(
                    v_flex()
                        .gap_3()
                        .child(preflight_item_row(&dialog_preflight.main, cx))
                        .when(!dialog_preflight.dependencies.is_empty(), |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .font_medium()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("必需前置"),
                            )
                            .children(
                                dialog_preflight
                                    .dependencies
                                    .iter()
                                    .map(|item| preflight_item_row(item, cx)),
                            )
                        })
                        .when(dialog_preflight.optional_count > 0, |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "另有 {} 个可选前置未自动安装。",
                                        dialog_preflight.optional_count
                                    )),
                            )
                        })
                        .when(!dialog_preflight.warnings.is_empty(), |this| {
                            this.child(
                                v_flex()
                                    .gap_1()
                                    .rounded(cx.theme().radius)
                                    .bg(cx.theme().warning.opacity(0.08))
                                    .p_3()
                                    .children(dialog_preflight.warnings.iter().map(|warning| {
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().warning)
                                            .child(warning.clone())
                                    })),
                            )
                        }),
                )
                .footer(
                    DialogFooter::new()
                        .child(DialogClose::new().trigger(|button| button.label("取消").outline()))
                        .child(
                            DialogAction::new().child(
                                Button::new("confirm-content-preflight")
                                    .primary()
                                    .label("安装"),
                            ),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    let Some(view) = save_view.upgrade() else {
                        return true;
                    };
                    view.update(cx, |this, cx| {
                        this.start_content_install_task(
                            save_request.clone(),
                            save_instance.clone(),
                            save_name.clone(),
                            window,
                            cx,
                        );
                    });
                    true
                })
        });
    }

    fn load_instance_content(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.content_request = self.content_request.wrapping_add(1);
        let request = self.content_request;
        let Some(version) = self.selected_instance_version().cloned() else {
            self.content_loading = false;
            self.content_library = None;
            self.content_error = Some("实例不存在或已被删除。".to_owned());
            cx.notify();
            return;
        };
        let instance = version.instance_layout(self.config.minecraft_root.clone());
        let services = self.services.clone();
        self.content_loading = true;
        self.content_error = None;
        self._content_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = services
                .runtime
                .spawn(async move { scan_instance_content(instance).await })
                .await;
            let _ = this.update_in(cx, |view, _, cx| {
                if view.content_request != request {
                    return;
                }
                view.content_loading = false;
                match result {
                    Ok(Ok(library)) => {
                        view.content_library = Some(library);
                        view.content_error = None;
                    }
                    Ok(Err(error)) => view.content_error = Some(error.to_string()),
                    Err(error) => {
                        view.content_error = Some(format!("后台任务失败: {error}"));
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn pick_content_files(
        &mut self,
        kind: ContentKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.content_operations_pending() {
            return;
        }
        let Some(version) = self.selected_instance_version().cloned() else {
            return;
        };
        let directory =
            kind.directory(&version.instance_layout(self.config.minecraft_root.clone()));
        let _ = std::fs::create_dir_all(&directory);

        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(format!("选择 {} 文件", kind.label()).into()),
        });
        self._prompt_task = Some(
            cx.spawn_in(window, async move |this, cx| match prompt.await {
                Ok(Ok(Some(paths))) if !paths.is_empty() => {
                    let _ = this.update_in(cx, |view, window, cx| {
                        view.import_content_files(kind, paths, window, cx)
                    });
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    let _ = this.update_in(cx, |_, window, cx| {
                        window.push_notification(
                            Notification::error(format!("无法打开文件选择器: {error}"))
                                .title("选择文件失败"),
                            cx,
                        );
                    });
                }
                Err(_) => {
                    let _ = this.update_in(cx, |_, window, cx| {
                        window.push_notification(
                            Notification::error("文件选择器已取消或窗口已关闭。")
                                .title("选择文件失败"),
                            cx,
                        );
                    });
                }
            }),
        );
        cx.notify();
    }

    fn import_content_files(
        &mut self,
        kind: ContentKind,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() || self.content_operations_pending() {
            return;
        }
        let Some(version) = self.selected_instance_version().cloned() else {
            return;
        };
        let instance = version.instance_layout(self.config.minecraft_root.clone());
        let services = self.services.clone();
        self.content_action_pending = true;
        self.content_error = None;
        self._content_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = services
                .runtime
                .spawn(async move {
                    let mut imported = 0usize;
                    let mut failures = Vec::new();
                    for source in paths {
                        match import_instance_content(
                            instance.clone(),
                            kind,
                            source,
                            ConflictPolicy::Rename,
                        )
                        .await
                        {
                            Ok(_) => imported += 1,
                            Err(error) => failures.push(error.to_string()),
                        }
                    }
                    let library = scan_instance_content(instance).await;
                    (imported, failures, library)
                })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.content_action_pending = false;
                match result {
                    Ok((imported, failures, library)) => {
                        match library {
                            Ok(library) => view.content_library = Some(library),
                            Err(error) => view.content_error = Some(error.to_string()),
                        }
                        if failures.is_empty() {
                            window.push_notification(
                                Notification::success(format!(
                                    "已导入 {imported} 个 {} 文件。",
                                    kind.label()
                                ))
                                .title("导入完成"),
                                cx,
                            );
                        } else {
                            window.push_notification(
                                Notification::warning(format!(
                                    "已导入 {imported} 个文件，{} 个失败：{}",
                                    failures.len(),
                                    failures.join("；")
                                ))
                                .title("部分导入失败"),
                                cx,
                            );
                        }
                    }
                    Err(error) => {
                        window.push_notification(
                            Notification::error(format!("后台任务失败: {error}")).title("导入失败"),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn toggle_content_item(
        &mut self,
        kind: ContentKind,
        relative_path: RepositoryPath,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.content_operations_pending() {
            return;
        }
        let Some(version) = self.selected_instance_version().cloned() else {
            return;
        };
        let instance = version.instance_layout(self.config.minecraft_root.clone());
        let services = self.services.clone();
        self.content_action_pending = true;
        self._content_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = services
                .runtime
                .spawn(async move {
                    let changed = set_instance_content_enabled(
                        instance.clone(),
                        kind,
                        relative_path,
                        enabled,
                    )
                    .await;
                    let library = scan_instance_content(instance).await;
                    (changed, library)
                })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.content_action_pending = false;
                match result {
                    Ok((changed, library)) => {
                        if let Err(error) = changed {
                            window.push_notification(
                                Notification::error(error.to_string()).title("更新内容失败"),
                                cx,
                            );
                        }
                        match library {
                            Ok(library) => view.content_library = Some(library),
                            Err(error) => view.content_error = Some(error.to_string()),
                        }
                    }
                    Err(error) => {
                        window.push_notification(
                            Notification::error(format!("后台任务失败: {error}"))
                                .title("更新内容失败"),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn confirm_delete_content(
        &mut self,
        kind: ContentKind,
        relative_path: RepositoryPath,
        display_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .confirm()
                .title(format!("删除 {display_name}？"))
                .description("将从实例中永久删除该内容条目。")
                .ok_text("删除")
                .ok_variant(ButtonVariant::Danger)
                .on_ok({
                    let relative_path = relative_path.clone();
                    let display_name = display_name.clone();
                    let view = view.clone();
                    move |_, window, cx| {
                        let Some(view) = view.upgrade() else {
                            return true;
                        };
                        view.update(cx, |this, cx| {
                            this.delete_content_item(
                                kind,
                                relative_path.clone(),
                                display_name.clone(),
                                window,
                                cx,
                            )
                        });
                        true
                    }
                })
        });
    }

    fn delete_content_item(
        &mut self,
        kind: ContentKind,
        relative_path: RepositoryPath,
        display_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.content_operations_pending() {
            return;
        }
        let Some(version) = self.selected_instance_version().cloned() else {
            return;
        };
        let instance = version.instance_layout(self.config.minecraft_root.clone());
        let services = self.services.clone();
        self.content_action_pending = true;
        self._content_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = services
                .runtime
                .spawn(async move {
                    let deleted =
                        delete_instance_content(instance.clone(), kind, relative_path).await;
                    let library = scan_instance_content(instance).await;
                    (deleted, library)
                })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.content_action_pending = false;
                match result {
                    Ok((deleted, library)) => match deleted {
                        Ok(()) => {
                            window.push_notification(
                                Notification::success(format!("已删除 {display_name}。"))
                                    .title("内容已删除"),
                                cx,
                            );
                            match library {
                                Ok(library) => view.content_library = Some(library),
                                Err(error) => view.content_error = Some(error.to_string()),
                            }
                        }
                        Err(error) => {
                            window.push_notification(
                                Notification::error(error.to_string()).title("删除内容失败"),
                                cx,
                            );
                        }
                    },
                    Err(error) => {
                        window.push_notification(
                            Notification::error(format!("后台任务失败: {error}"))
                                .title("删除内容失败"),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn open_content_folder(
        &mut self,
        kind: ContentKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(version) = self.selected_instance_version().cloned() else {
            return;
        };
        let directory =
            kind.directory(&version.instance_layout(self.config.minecraft_root.clone()));
        let _ = std::fs::create_dir_all(&directory);
        if let Err(error) = open_path(&directory) {
            window.push_notification(
                Notification::error(format!("无法打开内容目录: {error}")).title("打开失败"),
                cx,
            );
        }
    }

    fn filtered_content(&self, cx: &App) -> Vec<LocalContent> {
        let kind = self.content_tab.kind();
        let query = self.content_query.read(cx).value().to_lowercase();
        self.content_library
            .as_ref()
            .map(|library| {
                library
                    .items_for(kind)
                    .filter(|item| {
                        query.is_empty()
                            || item.display_name().to_lowercase().contains(&query)
                            || item.relative_path.as_str().to_lowercase().contains(&query)
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
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
                    && (query.is_empty()
                        || version.id.as_str().to_lowercase().contains(&query)
                        || version.name.to_lowercase().contains(&query)
                        || version.minecraft_version.to_lowercase().contains(&query))
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
            Page::Skin => self.render_skin(window, cx).into_any_element(),
            Page::Servers => self.render_servers(cx).into_any_element(),
            Page::Instance => self.render_instance(cx).into_any_element(),
            Page::CreateInstance => self.render_create_instance(cx).into_any_element(),
            Page::ContentBrowser => self.render_content_browser(cx).into_any_element(),
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
                    .items_stretch()
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

fn preflight_item_row(item: &ContentPreflightItem, cx: &App) -> AnyElement {
    let (label, variant) = match &item.status {
        ContentPreflightStatus::AlreadyInstalled { enabled: true } => ("已安装", Tag::success()),
        ContentPreflightStatus::AlreadyInstalled { enabled: false } => {
            ("已安装但禁用", Tag::warning())
        }
        ContentPreflightStatus::InstalledDifferentVersion { enabled: true, .. } => {
            ("已安装其他版本", Tag::warning())
        }
        ContentPreflightStatus::InstalledDifferentVersion { enabled: false, .. } => {
            ("已安装其他版本（禁用）", Tag::warning())
        }
        ContentPreflightStatus::WillInstall => ("将安装", Tag::info()),
        ContentPreflightStatus::Conflict => ("冲突", Tag::danger()),
    };
    h_flex()
        .w_full()
        .gap_3()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().border)
        .p_3()
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
                                .font_medium()
                                .truncate()
                                .child(item.title.clone()),
                        )
                        .child(variant.child(label).small()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{} · {}", item.version_number, item.file_name)),
                ),
        )
        .into_any_element()
}

fn updates_installable() -> bool {
    !cfg!(debug_assertions) || std::env::var_os("RMCL_ALLOW_DEV_UPDATE").is_some()
}

fn java_source_label(source: JavaRuntimeSource) -> &'static str {
    match source {
        JavaRuntimeSource::Managed => "RMCL 管理",
        JavaRuntimeSource::JavaHome => "JAVA_HOME",
        JavaRuntimeSource::System => "系统 PATH",
        JavaRuntimeSource::Custom => "自定义",
    }
}

type ServerPingResult = (
    String,
    Result<(ServerStatus, Option<PathBuf>), ServerPingError>,
);

async fn ping_servers(
    client: ServerStatusClient,
    icons: ServerIconCache,
    entries: Vec<(String, ServerAddress)>,
    cancellation: CancellationToken,
) -> Vec<ServerPingResult> {
    let mut queue = entries.into_iter();
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..SERVER_PING_CONCURRENCY {
        spawn_server_ping(&mut tasks, &mut queue, &client, &icons, &cancellation);
    }
    let mut results = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        if let Ok(result) = joined {
            results.push(result);
        }
        spawn_server_ping(&mut tasks, &mut queue, &client, &icons, &cancellation);
    }
    results
}

fn spawn_server_ping(
    tasks: &mut tokio::task::JoinSet<ServerPingResult>,
    queue: &mut impl Iterator<Item = (String, ServerAddress)>,
    client: &ServerStatusClient,
    icons: &ServerIconCache,
    cancellation: &CancellationToken,
) {
    let Some((id, address)) = queue.next() else {
        return;
    };
    let client = client.clone();
    let icons = icons.clone();
    let cancellation = cancellation.clone();
    tasks.spawn(async move {
        let result = async {
            let status = client.ping(&address, &cancellation).await?;
            let icon_path = match status.icon_png.as_ref() {
                Some(png) => Some(icons.ensure(png).await?),
                None => None,
            };
            Ok((status, icon_path))
        }
        .await;
        (id, result)
    });
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
    size: Rems,
    cx: &App,
) -> AnyElement {
    if let Some(head) = head {
        return div()
            .size(size)
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
        .size(size)
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

fn download_source_index(source: DownloadSourcePreference) -> usize {
    match source {
        DownloadSourcePreference::Auto => 0,
        DownloadSourcePreference::Official => 1,
        DownloadSourcePreference::Mirror => 2,
    }
}

fn mirror_preference(source: DownloadSourcePreference) -> MirrorPreference {
    match source {
        DownloadSourcePreference::Auto => MirrorPreference::Auto,
        DownloadSourcePreference::Official => MirrorPreference::Official,
        DownloadSourcePreference::Mirror => MirrorPreference::Mirror,
    }
}

fn initial_mirror_status(source: DownloadSourcePreference) -> String {
    match source {
        DownloadSourcePreference::Auto => "正在探测国内镜像…".to_owned(),
        DownloadSourcePreference::Official => "仅使用官方源".to_owned(),
        DownloadSourcePreference::Mirror => "国内镜像优先，正在探测…".to_owned(),
    }
}

fn mirror_probe_status(source: DownloadSourcePreference, probe: &MirrorProbe) -> String {
    match source {
        DownloadSourcePreference::Official => "仅使用官方源".to_owned(),
        DownloadSourcePreference::Auto => match (probe.ok, probe.latency) {
            (true, Some(latency)) => {
                format!("国内镜像可用（{} ms）", latency.as_millis())
            }
            _ => "国内镜像不可用，使用官方源".to_owned(),
        },
        DownloadSourcePreference::Mirror => match (probe.ok, probe.latency) {
            (true, Some(latency)) => {
                format!("国内镜像优先（{} ms）", latency.as_millis())
            }
            _ => "国内镜像优先；当前探测失败，将回退官方源".to_owned(),
        },
    }
}

fn spawn_mirror_probe(
    cx: &mut Context<LauncherApp>,
    runtime: tokio::runtime::Handle,
    mirror: Arc<MirrorHealth>,
    source: DownloadSourcePreference,
) -> Option<Task<()>> {
    if source == DownloadSourcePreference::Official {
        return None;
    }
    Some(cx.spawn(async move |this, cx| {
        // reqwest and `tokio::time::timeout` need the core runtime; the GPUI
        // executor does not provide a Tokio reactor.
        let probe = runtime
            .spawn(async move { mirror.probe_default(Duration::from_secs(2)).await })
            .await
            .unwrap_or_else(|error| MirrorProbe {
                ok: false,
                latency: None,
                error: Some(format!("mirror probe task failed: {error}")),
            });
        let status = mirror_probe_status(source, &probe);
        let _ = this.update(cx, |this, cx| {
            if this.config.download_source == source {
                this.mirror_status = status;
                cx.notify();
            }
        });
    }))
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
        Some(TaskProgress::Content(progress)) => {
            let file_ratio = progress
                .total_bytes
                .filter(|total| *total > 0)
                .map_or(0.0, |total| progress.downloaded_bytes as f32 / total as f32);
            if progress.total_files > 1 {
                (progress.completed_files as f32 + file_ratio) / progress.total_files as f32 * 100.0
            } else {
                file_ratio * 100.0
            }
        }
        Some(TaskProgress::Processors {
            completed, total, ..
        }) if *total > 0 => *completed as f32 / *total as f32 * 100.0,
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
        TaskProgress::Content(progress) => {
            let current = progress
                .current
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| progress.current.display().to_string());
            let position = if progress.total_files > 1 {
                format!(
                    "{}/{} · ",
                    progress.completed_files + 1,
                    progress.total_files
                )
            } else {
                String::new()
            };
            match progress.total_bytes {
                Some(total) => format!(
                    "{position}{current} · {} / {} 字节",
                    progress.downloaded_bytes, total
                ),
                None => format!("{position}{current} · {} 字节", progress.downloaded_bytes),
            }
        }
        TaskProgress::Processors {
            current,
            completed,
            total,
        } => format!("{completed} / {total} 个加载器处理器 · {current}"),
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
        TaskPhase::Install(InstallPhase::MaterializingAssets) => "展开旧版资源",
        TaskPhase::Install(InstallPhase::DownloadingLoader) => "下载加载器文件",
        TaskPhase::Install(InstallPhase::MergingNatives) => "合并 natives",
        TaskPhase::Install(InstallPhase::Completed) => "安装已完成",
        TaskPhase::InstanceCreate(InstanceCreatePhase::Preparing) => "准备实例",
        TaskPhase::InstanceCreate(InstanceCreatePhase::InstallingBase) => "安装基础版本",
        TaskPhase::InstanceCreate(InstanceCreatePhase::FetchingLoader) => "获取加载器元数据",
        TaskPhase::InstanceCreate(InstanceCreatePhase::WritingInstance) => "写入实例配置",
        TaskPhase::InstanceCreate(InstanceCreatePhase::CopyingContent) => "复制实例内容",
        TaskPhase::InstanceCreate(InstanceCreatePhase::InstallingLoader) => "安装实例文件",
        TaskPhase::InstanceCreate(InstanceCreatePhase::Completed) => "实例创建完成",
        TaskPhase::Content(ContentInstallPhase::Preparing) => "准备内容下载",
        TaskPhase::Content(ContentInstallPhase::Downloading) => "下载内容文件",
        TaskPhase::Content(ContentInstallPhase::Installing) => "写入实例内容",
        TaskPhase::Content(ContentInstallPhase::Completed) => "内容安装完成",
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
        Some(TaskStatus::Completed)
            if snapshot.map(|snapshot| snapshot.kind) == Some(TaskKind::InstanceCreate) =>
        {
            "实例创建完成".to_owned()
        }
        Some(TaskStatus::Completed)
            if snapshot.map(|snapshot| snapshot.kind) == Some(TaskKind::ContentInstall) =>
        {
            "内容安装完成".to_owned()
        }
        Some(TaskStatus::Completed) => "安装完成".to_owned(),
        Some(TaskStatus::Cancelled) => "已取消".to_owned(),
        Some(TaskStatus::Failed) => "任务失败".to_owned(),
        Some(TaskStatus::Exited) => "游戏已退出".to_owned(),
    }
}

pub fn task_kind_label(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::VanillaInstall => "安装",
        TaskKind::InstanceCreate => "创建实例",
        TaskKind::ContentInstall => "安装内容",
        TaskKind::GameLaunch => "启动",
    }
}

pub fn version_tile(icon: IconName, color: Hsla, cx: &App) -> impl IntoElement {
    div()
        .size_11()
        .flex_shrink_0()
        .rounded(cx.theme().radius_tokens().xl)
        .bg(color.opacity(0.12))
        .flex()
        .items_center()
        .justify_center()
        .child(Icon::new(icon).text_color(color).with_size(Size::Medium))
}

#[cfg(target_os = "macos")]
fn restore_macos_windows() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    let Some(marker) = MainThreadMarker::new() else {
        return;
    };
    let application = NSApplication::sharedApplication(marker);
    application.unhide(None);
    for window in application.windows().iter() {
        if window.isMiniaturized() {
            window.deminiaturize(None);
        }
    }
}
