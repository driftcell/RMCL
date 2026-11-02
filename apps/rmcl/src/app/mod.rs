mod agent;
mod external;
#[cfg(any(target_os = "macos", test))]
pub mod shortcuts;
mod skin_service;
mod skin_viewer;
mod ui;

use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon, IndexPath, Sizable, Size, StyledExt,
        VirtualListScrollHandle, WindowExt,
        button::{Button, ButtonVariant, ButtonVariants},
        carousel::CarouselState,
        combobox::{ComboboxEvent, ComboboxState},
        dialog::{DialogAction, DialogClose, DialogFooter},
        h_flex,
        input::{Input, InputEvent, InputState},
        notification::Notification,
        scroll::ScrollableElement,
        searchable_list::SearchableListItem,
        select::{SearchableVec, Select, SelectEvent, SelectState},
        spinner::Spinner,
        tag::Tag,
        text::{TextView, TextViewState},
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use rmcl_app::{
    AppError, AuthlibAccountServiceError, CatalogSkin, ConflictPolicy, ContentGraphInstallRequest,
    ContentInspection, ContentInstallPhase, ContentInstallPreflight, ContentKind, ContentLibrary,
    ContentPreflightItem, ContentPreflightStatus, ContentSource, CreateInstanceRequest,
    FeaturedServer, GameLaunchRequest, InstallPhase, InstalledVersion, InstanceCreatePhase,
    InstanceWorlds, JavaRuntime, JavaRuntimeSource, LoaderKind, LoaderSelection, LoaderVersion,
    LocalContent, MicrosoftAccountServiceError, RemoteProject, RemoteSearchQuery, RemoteSort,
    RemoteVersion, RepositoryPath, ResolvedAuthlibAccount, ResolvedMicrosoftAccount,
    ResourcePackOrder, ServerIconCache, ServerPingError, ServerStatus, ServerStatusClient,
    ShaderLoader, SkinCatalogSort, TaskEvent, TaskId, TaskKind, TaskPhase, TaskProgress,
    TaskSnapshot, TaskStatus, TaskSubject, UpdatePlan, UpdateProgress, UpdateRelease,
    VanillaInstallRequest, VersionManifest, VersionSource, VersionSummary, WorldCompatibility,
    WorldCopyProgress, WorldId, delete_installed_version, delete_instance_content,
    discover_installed_versions, featured_servers, import_instance_content,
    migrate_instance_to_isolated, move_instance_resource_pack, preflight_content_install,
    recommended_version, remove_server_bookmark, rename_instance,
    scan_and_inspect_instance_content, scan_installed_mods, set_instance_content_enabled,
    set_instance_java, set_instance_launch_settings, set_instance_shader_pack, spawn_apply_helper,
    upsert_server_bookmark,
};
use rmcl_auth::{
    AUTHLIB_SERVER_PRESETS, Account, AccountKind, AuthlibAccount, AuthlibError, AuthlibLogin,
    AuthlibProfile, MicrosoftAccount, MicrosoftAuthError, OfflineAccount,
};
use rmcl_domain::{Architecture, OperatingSystem, Platform, ServerAddress};
use rmcl_launch::{AuthlibLaunchConfig, GameLog, LaunchAccount, LogStream};
use rmcl_net::{MirrorHealth, MirrorPreference, MirrorProbe};
use rmcl_store::{
    AccentColor, CONFIG_SCHEMA_VERSION, DownloadSourcePreference, LanguagePreference,
    LauncherConfig, LauncherVisibility, StoredAccountKind, ThemePreference,
};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    i18n::tr,
    services::{
        Bootstrap, Services, load_account, load_authlib_account, load_microsoft_account, open_path,
        stored_account, stored_authlib_account, stored_microsoft_account,
    },
    state::LauncherState,
    theme,
};

use self::{
    external::merge_external_config,
    skin_service::{SkinService, SkinServiceEvent},
    skin_viewer::SkinViewer,
    ui::{
        common::{display_path, field_label, help, render_rgba_image},
        layout,
        skin::{ApplyState, LinkImport, LinkImportState, SkinEditor},
    },
};

const SERVER_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const SERVER_PING_CONCURRENCY: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SkinTab {
    #[default]
    Online,
    Local,
}

impl SkinTab {
    const ALL: [Self; 2] = [Self::Online, Self::Local];

    fn index(self) -> usize {
        match self {
            Self::Online => 0,
            Self::Local => 1,
        }
    }

    fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(Self::Online)
    }

    fn label(self) -> String {
        tr(match self {
            Self::Online => "app.skin.tab_online",
            Self::Local => "app.skin.tab_local",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Home,
    Agent,
    Versions,
    Worlds,
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
    #[cfg(any(target_os = "macos", test))]
    pub const NAVIGATION: [Self; 9] = [
        Self::Home,
        Self::Versions,
        Self::Worlds,
        Self::Skin,
        Self::Servers,
        Self::Downloads,
        Self::Agent,
        Self::Settings,
        Self::Logs,
    ];

    pub fn title(self) -> String {
        tr(match self {
            Self::Home => "app.page.home.title",
            Self::Agent => "app.page.agent.title",
            Self::Versions => "app.page.versions.title",
            Self::Worlds => "app.page.worlds.title",
            Self::Skin => "app.page.skin.title",
            Self::Servers => "app.page.servers.title",
            Self::Instance => "app.page.instance.title",
            Self::CreateInstance => "app.page.create_instance.title",
            Self::ContentBrowser => "app.page.content_browser.title",
            Self::Downloads => "app.page.downloads.title",
            Self::Logs => "app.page.logs.title",
            Self::Settings => "app.page.settings.title",
        })
    }

    pub fn subtitle(self) -> String {
        tr(match self {
            Self::Home => "app.page.home.subtitle",
            Self::Agent => "app.page.agent.subtitle",
            Self::Versions => "app.page.versions.subtitle",
            Self::Worlds => "app.page.worlds.subtitle",
            Self::Skin => "app.page.skin.subtitle",
            Self::Servers => "app.page.servers.subtitle",
            Self::Instance => "app.page.instance.subtitle",
            Self::CreateInstance => "app.page.create_instance.subtitle",
            Self::ContentBrowser => "app.page.content_browser.subtitle",
            Self::Downloads => "app.page.downloads.subtitle",
            Self::Logs => "app.page.logs.subtitle",
            Self::Settings => "app.page.settings.subtitle",
        })
    }

    fn icon(self) -> IconName {
        match self {
            Self::Home => IconName::House,
            Self::Agent => IconName::Bot,
            Self::Versions => IconName::Boxes,
            Self::Worlds => IconName::Map,
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

    pub fn label(self) -> String {
        tr(match self {
            Self::Favorites => "app.servers.tab_favorites",
            Self::Discover => "app.servers.tab_discover",
        })
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
    shader_loader: Option<ShaderLoader>,
    project: Option<RemoteProject>,
    projects: Vec<RemoteProject>,
    project_icons: HashMap<String, Option<PathBuf>>,
    project_previews: HashMap<String, Option<PathBuf>>,
    gallery: Option<Entity<CarouselState>>,
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

    pub fn label(self) -> String {
        content_kind_label(self.kind())
    }

    pub fn kind(self) -> ContentKind {
        match self {
            Self::Mods => ContentKind::Mod,
            Self::ResourcePacks => ContentKind::ResourcePack,
            Self::ShaderPacks => ContentKind::ShaderPack,
        }
    }

    pub fn from_kind(kind: ContentKind) -> Self {
        match kind {
            ContentKind::Mod => Self::Mods,
            ContentKind::ResourcePack => Self::ResourcePacks,
            ContentKind::ShaderPack => Self::ShaderPacks,
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

    pub fn label(self) -> String {
        tr(match self {
            Self::All => "app.version_filter.all",
            Self::Release => "app.version_filter.release",
            Self::Snapshot => "app.version_filter.snapshot",
            Self::Old => "app.version_filter.old",
        })
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

#[derive(Clone)]
enum AuthlibLoginUi {
    Requesting,
    SelectingProfile {
        login: Box<AuthlibLogin>,
        profiles: Vec<AuthlibProfile>,
    },
    Failed {
        message: String,
    },
}

pub struct LauncherApp {
    services: Arc<Services>,
    page: Page,
    config: LauncherConfig,
    config_warning: Option<String>,
    config_save_error: Option<String>,
    installed_versions: Vec<InstalledVersion>,
    versions_error: Option<String>,
    selected_instance: Option<String>,
    content_tab: ContentTab,
    content_library: Option<ContentLibrary>,
    content_inspection: Option<ContentInspection>,
    content_icons: HashMap<RepositoryPath, Arc<RenderImage>>,
    content_loading: bool,
    content_error: Option<String>,
    content_action_pending: bool,
    content_request: u64,
    content_query: Entity<InputState>,
    worlds: Option<InstanceWorlds>,
    worlds_instance: Option<String>,
    world_icons: HashMap<WorldId, Arc<RenderImage>>,
    world_query: Entity<InputState>,
    worlds_loading: bool,
    worlds_error: Option<String>,
    world_request: u64,
    world_action_pending: bool,
    world_progress: Option<WorldCopyProgress>,
    world_cancel: Option<CancellationToken>,
    _world_task: Option<Task<()>>,
    _world_load_task: Option<Task<()>>,
    _world_progress_task: Option<Task<()>>,
    content_browser: ContentBrowserState,
    agent: agent::AgentState,
    _agent_task: Option<Task<()>>,
    _agent_key_task: Option<Task<()>>,
    remote_query: Entity<InputState>,
    remote_sort: RemoteSort,
    content_browser_request: u64,
    content_browser_cancel: Option<CancellationToken>,
    content_browser_icons_cancel: Option<CancellationToken>,
    content_browser_previews_cancel: Option<CancellationToken>,
    _content_browser_task: Option<Task<()>>,
    _content_icons_task: Option<Task<()>>,
    _content_previews_task: Option<Task<()>>,
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
    skin_tab: SkinTab,
    skin_catalog_query: Entity<InputState>,
    skin_catalog_sort: SkinCatalogSort,
    skin_editor: SkinEditor,
    skin_link: LinkImport,
    skin_apply: ApplyState,
    pending_skin_profile: Option<MicrosoftAccount>,
    pending_catalog_skin: Option<CatalogSkin>,
    _skin_service_observe: Subscription,
    _skin_service_events: Subscription,
    _skin_link_subscription: Subscription,
    _skin_catalog_query_subscription: Subscription,
    version_select: Entity<SelectState<SearchableVec<String>>>,
    theme_select: Entity<SelectState<SearchableVec<String>>>,
    accent_select: Entity<SelectState<SearchableVec<String>>>,
    visibility_select: Entity<SelectState<SearchableVec<String>>>,
    download_source_select: Entity<SelectState<SearchableVec<String>>>,
    language_select: Entity<SelectState<SearchableVec<String>>>,
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
    gui_task_ids: HashSet<TaskId>,
    deferred_task_terminals: Vec<TaskSnapshot>,
    launcher_hidden: bool,
    logs_attention: bool,
    launch_resolving: bool,
    microsoft_login: Option<MicrosoftLoginUi>,
    login_cancel: Option<CancellationToken>,
    login_code_copied: bool,
    authlib_login: Option<AuthlibLoginUi>,
    authlib_server_select: Entity<SelectState<SearchableVec<String>>>,
    authlib_server_input: Entity<InputState>,
    authlib_username_input: Entity<InputState>,
    authlib_password_input: Entity<InputState>,
    authlib_custom_server: bool,
    authlib_login_cancel: Option<CancellationToken>,
    _authlib_login_task: Option<Task<()>>,
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
    _activation_subscription: Subscription,
}

impl LauncherApp {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        services: Arc<Services>,
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
        accounts.extend(
            config
                .authlib_accounts
                .iter()
                .filter_map(load_authlib_account)
                .map(Account::Authlib),
        );
        if !accounts.iter().any(|account| {
            config.selected_account_id.as_deref() == Some(account_key(account).as_str())
        }) {
            config.selected_account = accounts
                .first()
                .map(|account| account.username().to_owned());
            config.selected_account_kind = accounts
                .first()
                .map(stored_account_kind)
                .unwrap_or(StoredAccountKind::Offline);
            config.selected_account_id = accounts.first().map(account_key);
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
                SearchableVec::new(theme_items()),
                Some(IndexPath::default().row(theme_index(config.appearance.theme))),
                window,
                cx,
            )
        });
        let accent_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(accent_items()),
                Some(IndexPath::default().row(accent_index(config.appearance.accent))),
                window,
                cx,
            )
        });
        let visibility_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(visibility_items()),
                Some(IndexPath::default().row(visibility_index(config.launcher_visibility))),
                window,
                cx,
            )
        });
        let download_source_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(download_source_items()),
                Some(IndexPath::default().row(download_source_index(config.download_source))),
                window,
                cx,
            )
        });
        let language_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(language_items()),
                Some(IndexPath::default().row(language_index(config.language))),
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

        let downloads_query = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr("app.placeholder.search_versions"))
        });
        let versions_query = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr("app.placeholder.search_instances"))
        });
        let content_query = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr("app.placeholder.search_local_content"))
        });
        let world_query = cx
            .new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.search_worlds")));
        let remote_query = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr("app.placeholder.search_modrinth"))
        });
        let server_query = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr("app.placeholder.search_servers"))
        });
        let discover_query = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr("app.placeholder.search_featured"))
        });
        let server_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.server_name")));
        let server_address_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr("app.placeholder.server_address"))
        });
        let create_base_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr("app.placeholder.version_example"))
        });
        let create_version_select = cx.new(|cx| {
            ComboboxState::new(
                SearchableVec::<CreateVersionChoice>::new(Vec::new()),
                Vec::new(),
                window,
                cx,
            )
            .searchable(true)
        });
        let create_name_input = cx
            .new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.instance_name")));
        let create_id_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.instance_id")));
        let instance_name_input = cx
            .new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.instance_name")));
        let java_path_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.java_path")));
        let launch_memory_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.jvm_default")));
        let launch_width_input = cx.new(|cx| InputState::new(window, cx).placeholder("1280"));
        let launch_height_input = cx.new(|cx| InputState::new(window, cx).placeholder("720"));
        let account_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr("app.placeholder.offline_username"))
        });
        let authlib_server_items = authlib_server_items();
        let authlib_server_select = cx.new(|cx| {
            SelectState::new(
                authlib_server_items,
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
        });
        let authlib_server_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("https://example.com/api/yggdrasil/")
        });
        let authlib_username_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr("app.placeholder.email_or_username"))
        });
        let authlib_password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(tr("app.placeholder.password"))
                .masked(true)
        });
        let skin_link_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.png_url")));
        let skin_catalog_query = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr("app.placeholder.search_littleskin"))
        });

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
            cx.subscribe(&world_query, |_this, _, event: &InputEvent, cx| {
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
        subscriptions.push(cx.subscribe_in(
            &language_select,
            window,
            |this, _, event: &SelectEvent<SearchableVec<String>>, window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    this.set_language(value.clone(), window, cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe(
            &authlib_server_select,
            |this, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    this.authlib_custom_server = *value == authlib_custom_server_label();
                    cx.notify();
                }
            },
        ));
        for input in [
            &authlib_server_input,
            &authlib_username_input,
            &authlib_password_input,
        ] {
            subscriptions.push(cx.subscribe(input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    if matches!(this.authlib_login, Some(AuthlibLoginUi::Failed { .. })) {
                        this.authlib_login = None;
                    }
                    cx.notify();
                }
            }));
        }

        let skin_service_observe = cx.observe(&skin_service, |_, _, cx| cx.notify());
        let skin_service_events =
            cx.subscribe(&skin_service, |this, _, event: &SkinServiceEvent, cx| {
                match event {
                    SkinServiceEvent::ProfileLoaded(loaded) => {
                        this.pending_skin_profile = Some(loaded.profile.clone());
                    }
                    SkinServiceEvent::CatalogSkinLoaded(loaded) => {
                        this.pending_catalog_skin = Some(loaded.clone());
                    }
                }
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
        let skin_catalog_query_subscription =
            cx.subscribe(&skin_catalog_query, |_this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
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
        let activation_subscription = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.refresh_external_state(window, cx);
            }
        });

        let config_for_agent = config.clone();
        let mut app = Self {
            services: services.clone(),
            page: Page::Home,
            config,
            config_warning,
            config_save_error: None,
            installed_versions,
            versions_error,
            selected_instance: None,
            content_tab: ContentTab::Mods,
            content_library: None,
            content_inspection: None,
            content_icons: HashMap::new(),
            content_loading: false,
            content_error: None,
            content_action_pending: false,
            content_request: 0,
            content_query,
            worlds: None,
            worlds_instance: None,
            world_icons: HashMap::new(),
            world_query,
            worlds_loading: false,
            worlds_error: None,
            world_request: 0,
            world_action_pending: false,
            world_progress: None,
            world_cancel: None,
            _world_task: None,
            _world_load_task: None,
            _world_progress_task: None,
            content_browser: ContentBrowserState::default(),
            agent: agent::AgentState::new(window, cx, &config_for_agent),
            _agent_task: None,
            _agent_key_task: None,
            remote_query,
            remote_sort: RemoteSort::Relevance,
            content_browser_request: 0,
            content_browser_cancel: None,
            content_browser_icons_cancel: None,
            content_browser_previews_cancel: None,
            _content_browser_task: None,
            _content_icons_task: None,
            _content_previews_task: None,
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
            skin_tab: SkinTab::default(),
            skin_catalog_query,
            skin_catalog_sort: SkinCatalogSort::default(),
            skin_editor: SkinEditor::default(),
            skin_link: LinkImport::new(skin_link_input),
            skin_apply: ApplyState::Idle,
            pending_skin_profile: None,
            pending_catalog_skin: None,
            _skin_service_observe: skin_service_observe,
            _skin_service_events: skin_service_events,
            _skin_link_subscription: skin_link_subscription,
            _skin_catalog_query_subscription: skin_catalog_query_subscription,
            version_select,
            theme_select,
            accent_select,
            visibility_select,
            download_source_select,
            language_select,
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
            gui_task_ids: HashSet::new(),
            deferred_task_terminals: Vec::new(),
            launcher_hidden: false,
            logs_attention: false,
            launch_resolving: false,
            microsoft_login: None,
            login_cancel: None,
            login_code_copied: false,
            authlib_login: None,
            authlib_server_select,
            authlib_server_input,
            authlib_username_input,
            authlib_password_input,
            authlib_custom_server: false,
            authlib_login_cancel: None,
            _authlib_login_task: None,
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
            _activation_subscription: activation_subscription,
        };
        let agent_input = app.agent.conversation.input.clone();
        app._subscriptions.push(cx.subscribe_in(
            &agent_input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(
                    event,
                    InputEvent::PressEnter {
                        secondary: false,
                        shift: false
                    }
                ) {
                    this.send_agent_message(window, cx);
                }
            },
        ));
        let agent_provider_select = app.agent.settings.provider_select.clone();
        app._subscriptions.push(cx.subscribe_in(
            &agent_provider_select,
            window,
            |this, _, event: &SelectEvent<SearchableVec<String>>, window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    this.set_agent_provider(value, window, cx);
                }
            },
        ));
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
                                Notification::info(
                                    rust_i18n::t!(
                                        "app.updates.available_notice",
                                        version = version
                                    )
                                    .to_string(),
                                )
                                .title(tr("app.updates.title")),
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
                            view.update.error = Some(
                                rust_i18n::t!("app.common.background_task_failed", error = error)
                                    .to_string(),
                            );
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
                Notification::warning(tr("app.updates.dev_build")).title(tr("app.updates.title")),
                cx,
            );
            return;
        }
        let view = cx.entity().downgrade();
        let dialog_release = release.clone();
        let notes_view = dialog_release
            .notes
            .clone()
            .filter(|notes| !notes.trim().is_empty())
            .map(|notes| cx.new(|cx| TextViewState::markdown(&notes, cx)));
        window.open_dialog(cx, move |dialog, _window, cx| {
            let save_view = view.clone();
            let save_release = dialog_release.clone();
            dialog
                .title(
                    rust_i18n::t!("app.updates.dialog_title", version = dialog_release.version)
                        .to_string(),
                )
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
                                .w_full()
                                .min_w_0()
                                .max_h(rems(15.))
                                .overflow_y_scrollbar()
                                .when_some(notes_view.as_ref(), |this, notes| {
                                    this.child(TextView::new(notes).selectable(true))
                                })
                                .when(notes_view.is_none(), |this| {
                                    this.child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(tr("app.updates.no_notes")),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr("app.updates.verify_notice")),
                        ),
                )
                .footer(
                    DialogFooter::new()
                        .child(
                            DialogClose::new()
                                .trigger(|button| button.label(tr("app.common.cancel")).outline()),
                        )
                        .child(
                            DialogAction::new().child(
                                Button::new("confirm-update")
                                    .primary()
                                    .label(tr("app.updates.download_and_install")),
                            ),
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
                Notification::warning(tr("app.updates.dev_build")).title(tr("app.updates.title")),
                cx,
            );
            return;
        }
        if !self.can_start_operation() {
            window.push_notification(
                Notification::warning(tr("app.updates.wait_for_task"))
                    .title(tr("app.updates.title")),
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
                                                    Notification::info(tr("app.updates.restarting"))
                                                        .title(tr("app.updates.updating")),
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
        services: &Arc<Services>,
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

    fn select_account_key(&mut self, account_id: &str, cx: &mut Context<Self>) {
        if self.config.selected_account_id.as_deref() == Some(account_id) {
            return;
        }
        let Some(account) = self
            .accounts
            .iter()
            .find(|account| account_key(account) == account_id)
        else {
            return;
        };
        self.config.selected_account = Some(account.username().to_owned());
        self.config.selected_account_kind = stored_account_kind(account);
        self.config.selected_account_id = Some(account_id.to_owned());
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
        let theme = if value == theme_label(ThemePreference::System) {
            ThemePreference::System
        } else if value == theme_label(ThemePreference::Light) {
            ThemePreference::Light
        } else if value == theme_label(ThemePreference::Dark) {
            ThemePreference::Dark
        } else {
            return;
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
        let accent = if value == accent_label(AccentColor::Emerald) {
            AccentColor::Emerald
        } else if value == accent_label(AccentColor::Ocean) {
            AccentColor::Ocean
        } else if value == accent_label(AccentColor::Violet) {
            AccentColor::Violet
        } else if value == accent_label(AccentColor::Amber) {
            AccentColor::Amber
        } else if value == accent_label(AccentColor::Rose) {
            AccentColor::Rose
        } else {
            return;
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
        let visibility = if value == visibility_label(LauncherVisibility::KeepVisible) {
            LauncherVisibility::KeepVisible
        } else if value == visibility_label(LauncherVisibility::HideWhileRunning) {
            LauncherVisibility::HideWhileRunning
        } else {
            return;
        };
        if self.config.launcher_visibility == visibility {
            return;
        }
        self.config.launcher_visibility = visibility;
        self.save_config();
        cx.notify();
    }

    fn set_download_source(&mut self, value: String, cx: &mut Context<Self>) {
        let source = if value == download_source_label(DownloadSourcePreference::Auto) {
            DownloadSourcePreference::Auto
        } else if value == download_source_label(DownloadSourcePreference::Official) {
            DownloadSourcePreference::Official
        } else if value == download_source_label(DownloadSourcePreference::Mirror) {
            DownloadSourcePreference::Mirror
        } else {
            return;
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

    fn set_language(&mut self, value: String, window: &mut Window, cx: &mut Context<Self>) {
        let language = if value == language_label(LanguagePreference::System) {
            LanguagePreference::System
        } else if value == language_label(LanguagePreference::Chinese) {
            LanguagePreference::Chinese
        } else if value == language_label(LanguagePreference::English) {
            LanguagePreference::English
        } else {
            return;
        };
        if self.config.language == language {
            return;
        }
        self.config.language = language;
        crate::i18n::apply(language);
        self.mirror_status = initial_mirror_status(self.config.download_source);
        self.save_config();
        self.refresh_localized_controls(window, cx);
        #[cfg(target_os = "macos")]
        self::shortcuts::refresh_menus(cx);
        cx.refresh_windows();
        cx.notify();
    }

    /// Rebuilds select labels that are captured when the entity is created.
    fn refresh_localized_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let theme = self.config.appearance.theme;
        self.theme_select.update(cx, |state, cx| {
            state.set_items(SearchableVec::new(theme_items()), window, cx);
            state.set_selected_index(
                Some(IndexPath::default().row(theme_index(theme))),
                window,
                cx,
            );
        });
        let accent = self.config.appearance.accent;
        self.accent_select.update(cx, |state, cx| {
            state.set_items(SearchableVec::new(accent_items()), window, cx);
            state.set_selected_index(
                Some(IndexPath::default().row(accent_index(accent))),
                window,
                cx,
            );
        });
        let visibility = self.config.launcher_visibility;
        self.visibility_select.update(cx, |state, cx| {
            state.set_items(SearchableVec::new(visibility_items()), window, cx);
            state.set_selected_index(
                Some(IndexPath::default().row(visibility_index(visibility))),
                window,
                cx,
            );
        });
        let source = self.config.download_source;
        self.download_source_select.update(cx, |state, cx| {
            state.set_items(SearchableVec::new(download_source_items()), window, cx);
            state.set_selected_index(
                Some(IndexPath::default().row(download_source_index(source))),
                window,
                cx,
            );
        });
        self.agent.refresh_provider_labels(window, cx);
        let language = self.config.language;
        self.language_select.update(cx, |state, cx| {
            state.set_items(SearchableVec::new(language_items()), window, cx);
            state.set_selected_index(
                Some(IndexPath::default().row(language_index(language))),
                window,
                cx,
            );
        });
        let authlib_custom = self.authlib_custom_server;
        let authlib_row = self
            .authlib_server_select
            .read(cx)
            .selected_index(cx)
            .map(|index| index.row)
            .unwrap_or(0);
        self.authlib_server_select.update(cx, |state, cx| {
            state.set_items(authlib_server_items(), window, cx);
            let row = if authlib_custom {
                AUTHLIB_SERVER_PRESETS.len()
            } else {
                authlib_row.min(AUTHLIB_SERVER_PRESETS.len().saturating_sub(1))
            };
            state.set_selected_index(Some(IndexPath::default().row(row)), window, cx);
        });
        for (input, key) in [
            (&self.downloads_query, "app.placeholder.search_versions"),
            (&self.versions_query, "app.placeholder.search_instances"),
            (&self.content_query, "app.placeholder.search_local_content"),
            (&self.world_query, "app.placeholder.search_worlds"),
            (&self.remote_query, "app.placeholder.search_modrinth"),
            (&self.server_query, "app.placeholder.search_servers"),
            (&self.discover_query, "app.placeholder.search_featured"),
            (&self.server_name_input, "app.placeholder.server_name"),
            (&self.server_address_input, "app.placeholder.server_address"),
            (&self.create_base_input, "app.placeholder.version_example"),
            (&self.create_name_input, "app.placeholder.instance_name"),
            (&self.create_id_input, "app.placeholder.instance_id"),
            (&self.instance_name_input, "app.placeholder.instance_name"),
            (&self.java_path_input, "app.placeholder.java_path"),
            (&self.launch_memory_input, "app.placeholder.jvm_default"),
            (&self.account_input, "app.placeholder.offline_username"),
            (
                &self.authlib_username_input,
                "app.placeholder.email_or_username",
            ),
            (&self.authlib_password_input, "app.placeholder.password"),
            (&self.skin_link.input, "app.placeholder.png_url"),
            (
                &self.skin_catalog_query,
                "app.placeholder.search_littleskin",
            ),
        ] {
            input.update(cx, |state, cx| {
                state.set_placeholder(tr(key), window, cx);
            });
        }
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
                Account::Microsoft(_) | Account::Authlib(_) => None,
            })
            .collect();
        self.config.microsoft_accounts = self
            .accounts
            .iter()
            .filter_map(|account| match account {
                Account::Microsoft(account) => Some(stored_microsoft_account(account)),
                Account::Offline(_) | Account::Authlib(_) => None,
            })
            .collect();
        self.config.authlib_accounts = self
            .accounts
            .iter()
            .filter_map(|account| match account {
                Account::Authlib(account) => Some(stored_authlib_account(account)),
                Account::Offline(_) | Account::Microsoft(_) => None,
            })
            .collect();
        let selected_exists = self
            .config
            .selected_account_id
            .as_deref()
            .is_some_and(|id| {
                self.accounts
                    .iter()
                    .any(|account| account_key(account) == id)
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
            self.config.selected_account_id = self.accounts.first().map(account_key);
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
        if self.page == Page::Worlds {
            self.load_worlds(window, cx);
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

    fn refresh_external_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.services.config_store.load() {
            Ok(external) => {
                self.config_warning = None;
                if merge_external_config(&mut self.config, external) {
                    self.accounts = self
                        .config
                        .accounts
                        .iter()
                        .filter_map(load_account)
                        .map(Account::Offline)
                        .chain(
                            self.config
                                .microsoft_accounts
                                .iter()
                                .filter_map(load_microsoft_account)
                                .map(Account::Microsoft),
                        )
                        .chain(
                            self.config
                                .authlib_accounts
                                .iter()
                                .filter_map(load_authlib_account)
                                .map(Account::Authlib),
                        )
                        .collect::<Vec<_>>();
                    let selected_exists =
                        self.config
                            .selected_account_id
                            .as_deref()
                            .is_some_and(|id| {
                                self.accounts
                                    .iter()
                                    .any(|account| account_key(account) == id)
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
                        self.config.selected_account_id = self.accounts.first().map(account_key);
                    }
                    self.refresh_account_heads(window, cx);
                }
            }
            Err(error) => self.config_warning = Some(error.to_string()),
        }
        self.reload_versions(window, cx);
        if self.page == Page::Instance && !self.content_operations_pending() {
            self.load_instance_content(window, cx);
        }
    }

    fn selected_account(&self) -> Option<&Account> {
        if let Some(id) = self.config.selected_account_id.as_deref() {
            return self
                .accounts
                .iter()
                .find(|account| account_key(account) == id);
        }
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

    fn server_join_ready(&self) -> (bool, String) {
        if self.selected_version().is_none() {
            (false, tr("app.servers.join_requires_instance"))
        } else if self.selected_account().is_none() {
            (false, tr("app.servers.join_requires_account"))
        } else if !self.can_start_operation() {
            (false, tr("app.servers.join_busy"))
        } else {
            (true, tr("app.servers.join_ready"))
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
                    status.error = Some(
                        rust_i18n::t!("app.servers.invalid_address", error = error).to_string(),
                    );
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
                            status.error = Some(
                                rust_i18n::t!("app.common.background_task_failed", error = error)
                                    .to_string(),
                            );
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
                                status.error = Some(
                                    rust_i18n::t!(
                                        "app.common.background_task_failed",
                                        error = error
                                    )
                                    .to_string(),
                                );
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
            window.push_notification(
                Notification::info(tr("app.servers.unfavorited"))
                    .title(tr("app.page.servers.title")),
                cx,
            );
            return;
        }
        if let Err(error) = upsert_server_bookmark(
            &mut self.config,
            None,
            &server.name,
            &server.address.to_string(),
        ) {
            window.push_notification(
                Notification::error(error.to_string()).title(tr("app.servers.favorite_failed")),
                cx,
            );
            return;
        }
        self.save_config();
        cx.notify();
        window.push_notification(
            Notification::success(tr("app.servers.favorited")).title(tr("app.page.servers.title")),
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
                Notification::warning(tr("app.servers.modpack_required_notice"))
                    .title(tr("app.servers.requires_modpack")),
                cx,
            );
            return;
        }
        let Some(version) = self.selected_version() else {
            window.push_notification(
                Notification::warning(tr("app.servers.select_instance_notice"))
                    .title(tr("app.servers.cannot_join")),
                cx,
            );
            return;
        };
        let version_id = version.id.as_str().to_owned();
        if self.selected_account().is_none() {
            window.push_notification(
                Notification::warning(tr("app.servers.add_account_notice"))
                    .title(tr("app.servers.cannot_join")),
                cx,
            );
            return;
        }
        if !self.can_start_operation() {
            window.push_notification(
                Notification::warning(tr("app.servers.busy_notice"))
                    .title(tr("app.servers.cannot_join")),
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
                .title(if is_edit {
                    tr("app.servers.edit")
                } else {
                    tr("app.servers.add")
                })
                .width(layout::rem_px(cx, layout::DIALOG_LARGE))
                .child(
                    v_flex()
                        .gap_4()
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_sm().font_medium().child(tr("app.servers.name")))
                                .child(div().child(Input::new(&name_input).w_full()))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(tr("app.servers.name_hint")),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_medium()
                                        .child(tr("app.servers.address")),
                                )
                                .child(div().child(Input::new(&address_input).w_full()))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(tr("app.servers.address_hint")),
                                ),
                        ),
                )
                .footer(
                    DialogFooter::new()
                        .child(
                            DialogClose::new()
                                .trigger(|button| button.label(tr("app.common.cancel")).outline()),
                        )
                        .child(
                            DialogAction::new().child(
                                Button::new("save-server")
                                    .primary()
                                    .label(tr("app.common.save")),
                            ),
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
        if let Err(error) = upsert_server_bookmark(&mut self.config, id, &name, &address) {
            window.push_notification(
                Notification::error(error.to_string()).title(tr("app.servers.save_failed")),
                cx,
            );
            return false;
        }
        self.cancel_server_refresh();
        self.save_config();
        self.refresh_servers(window, cx);
        cx.notify();
        window.push_notification(
            Notification::success(tr("app.servers.saved")).title(tr("app.page.servers.title")),
            cx,
        );
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
                .title(rust_i18n::t!("app.servers.delete_title", name = server.name).to_string())
                .description(tr("app.servers.delete_description"))
                .ok_text(tr("app.common.delete"))
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
        if remove_server_bookmark(&mut self.config, &server_id).is_err() {
            return;
        }
        self.cancel_server_refresh();
        self.servers.statuses.remove(&server_id);
        self.save_config();
        self.refresh_servers(window, cx);
        cx.notify();
        window.push_notification(
            Notification::info(tr("app.servers.deleted")).title(tr("app.page.servers.title")),
            cx,
        );
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
                    Notification::error(error.to_string()).title(tr("app.servers.cannot_join")),
                    cx,
                );
                return;
            }
        };
        let Some(version) = self.selected_version() else {
            window.push_notification(
                Notification::warning(tr("app.servers.select_instance_notice"))
                    .title(tr("app.servers.cannot_join")),
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
        if page == Page::Worlds {
            self.load_worlds(window, cx);
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
        if matches!(page, Page::Agent | Page::Settings) {
            self.refresh_agent_key_state(cx);
        }
        if page == Page::Agent {
            let scroller = self.agent.conversation.scroller.clone();
            let count = scroller.read(cx).item_count();
            if count > 1 {
                let following = scroller.read(cx).is_following_tail();
                scroller.update(cx, |scroller, cx| {
                    let _ = scroller.remeasure_items(count - 2..count, cx);
                    if following {
                        scroller.scroll_to_end(cx);
                    }
                });
            }
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
                            Notification::error(message).title(tr("app.downloads.load_failed")),
                            cx,
                        );
                    }
                    Err(error) => {
                        let message =
                            rust_i18n::t!("app.common.background_task_failed", error = error)
                                .to_string();
                        view.catalog.error = Some(message.clone());
                        window.push_notification(
                            Notification::error(message).title(tr("app.downloads.load_failed")),
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
                Notification::warning(tr("app.common.operation_busy"))
                    .title(tr("app.install.cannot_start")),
                cx,
            );
            return;
        }
        window.push_notification(
            Notification::info(
                rust_i18n::t!("app.install.checking", version = version_id).to_string(),
            )
            .title(tr("app.install.started")),
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
                Notification::warning(tr("app.common.operation_busy"))
                    .title(tr("app.content_install.cannot_start")),
                cx,
            );
            return;
        }
        window.push_notification(
            Notification::info(
                rust_i18n::t!("app.content_install.installing", name = display_name).to_string(),
            )
            .title(tr("app.content_install.started")),
            cx,
        );
        self.spawn_command(Command::ContentInstall(request), instance_id, window, cx);
    }

    fn start_launch(&mut self, version_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.start_launch_with_target(version_id, None, None, window, cx);
    }

    fn start_launch_with_server(
        &mut self,
        version_id: String,
        server: Option<ServerAddress>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_launch_with_target(version_id, server, None, window, cx);
    }

    fn start_launch_with_world(
        &mut self,
        version_id: String,
        world: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_launch_with_target(version_id, None, Some(world), window, cx);
    }

    fn start_launch_with_target(
        &mut self,
        version_id: String,
        server: Option<ServerAddress>,
        quick_play_world: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.launch_resolving {
            return;
        }
        if !self.can_start_operation() {
            window.push_notification(
                Notification::warning(tr("app.common.operation_busy"))
                    .title(tr("app.launch.cannot_start")),
                cx,
            );
            return;
        }
        let Some(account) = self.selected_account().cloned() else {
            window.push_notification(
                Notification::error(tr("app.launch.account_required"))
                    .title(tr("app.launch.cannot_start")),
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
                    quick_play_world,
                    None,
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
                                        Notification::warning(warning.clone())
                                            .title(tr("app.account.title")),
                                        cx,
                                    );
                                }
                                if !view.can_start_operation() {
                                    window.push_notification(
                                        Notification::warning(tr("app.common.operation_busy"))
                                            .title(tr("app.launch.cannot_start")),
                                        cx,
                                    );
                                } else {
                                    view.spawn_launch(
                                        resolved.launch,
                                        version_id,
                                        server,
                                        quick_play_world,
                                        None,
                                        window,
                                        cx,
                                    );
                                }
                            }
                            Ok(Err(error)) => {
                                window.push_notification(
                                    Notification::error(microsoft_error_message(&error))
                                        .title(tr("app.launch.cannot_start")),
                                    cx,
                                );
                            }
                            Err(error) => {
                                window.push_notification(
                                    Notification::error(
                                        rust_i18n::t!(
                                            "app.common.background_task_failed",
                                            error = error
                                        )
                                        .to_string(),
                                    )
                                    .title(tr("app.launch.cannot_start")),
                                    cx,
                                );
                            }
                        }
                        cx.notify();
                    });
                }));
            }
            Account::Authlib(account) => {
                self.launch_resolving = true;
                cx.notify();
                let service = self.services.authlib_accounts.clone();
                let injector = self.services.authlib_injector.clone();
                let runtime = self.services.runtime.handle().clone();
                let cancellation = CancellationToken::new();
                self._launch_task = Some(cx.spawn_in(window, async move |this, cx| {
                    let result = runtime
                        .spawn(async move {
                            let resolved = service
                                .resolve_launch_account(&account, &cancellation)
                                .await?;
                            let artifact = injector.ensure(&cancellation).await?;
                            Ok::<_, AppError>((resolved, artifact))
                        })
                        .await;
                    let _ = this.update_in(cx, move |view, window, cx| {
                        view.launch_resolving = false;
                        match result {
                            Ok(Ok((resolved, artifact))) => {
                                view.update_resolved_authlib(&resolved, window, cx);
                                if let Some(warning) = resolved.persistence_warning.as_ref() {
                                    window.push_notification(
                                        Notification::warning(warning.clone())
                                            .title(tr("app.account.title")),
                                        cx,
                                    );
                                }
                                if !view.can_start_operation() {
                                    window.push_notification(
                                        Notification::warning(tr("app.common.operation_busy"))
                                            .title(tr("app.launch.cannot_start")),
                                        cx,
                                    );
                                } else {
                                    let authlib = AuthlibLaunchConfig {
                                        jar_path: artifact.path,
                                        api_root: resolved.account.api_root().to_owned(),
                                        prefetched_metadata: resolved.prefetched_metadata.clone(),
                                    };
                                    view.spawn_launch(
                                        resolved.launch,
                                        version_id,
                                        server,
                                        quick_play_world,
                                        Some(authlib),
                                        window,
                                        cx,
                                    );
                                }
                            }
                            Ok(Err(error)) => {
                                window.push_notification(
                                    Notification::error(authlib_launch_error_message(&error))
                                        .title(tr("app.launch.cannot_start")),
                                    cx,
                                );
                            }
                            Err(error) => {
                                window.push_notification(
                                    Notification::error(
                                        rust_i18n::t!(
                                            "app.common.background_task_failed",
                                            error = error
                                        )
                                        .to_string(),
                                    )
                                    .title(tr("app.launch.cannot_start")),
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

    #[allow(clippy::too_many_arguments)]
    fn spawn_launch(
        &mut self,
        account: LaunchAccount,
        version_id: String,
        server: Option<ServerAddress>,
        quick_play_world: Option<String>,
        authlib: Option<AuthlibLaunchConfig>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_start_operation() {
            window.push_notification(
                Notification::warning(tr("app.common.operation_busy"))
                    .title(tr("app.launch.cannot_start")),
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
            authlib,
            minecraft_version,
            server,
            quick_play_world,
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

    fn update_resolved_authlib(
        &mut self,
        resolved: &ResolvedAuthlibAccount,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = resolved.account.id();
        if let Some(existing) = self
            .accounts
            .iter_mut()
            .find(|account| matches!(account, Account::Authlib(account) if account.id() == id))
        {
            *existing = Account::Authlib(resolved.account.clone());
        } else {
            self.accounts
                .push(Account::Authlib(resolved.account.clone()));
        }
        if self.config.selected_account_id.as_deref() == Some(format!("authlib:{id}").as_str()) {
            self.config.selected_account = Some(resolved.account.username().to_owned());
        }
        self.save_config();
        cx.notify();
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
                        view.gui_task_ids.insert(id);
                        view.deferred_task_terminals.retain(|task| task.id != id);
                        view.log_state.complete_start(id, snapshot.clone(), logs);
                        if let Some(snapshot) = snapshot
                            && snapshot.status.is_terminal()
                        {
                            view.on_task_terminal(snapshot, window, cx);
                        }
                        view.replay_deferred_task_terminals(window, cx);
                        cx.notify();
                    });
                }
                Ok(Err(error)) => {
                    let message = error.to_string();
                    let _ = this.update_in(cx, |view, window, cx| {
                        view.log_state.fail_command(message.clone());
                        window.push_notification(
                            Notification::error(message).title(tr("app.task.start_failed")),
                            cx,
                        );
                        view.replay_deferred_task_terminals(window, cx);
                        cx.notify();
                    });
                }
                Err(error) => {
                    let message = rust_i18n::t!("app.common.background_task_failed", error = error)
                        .to_string();
                    let _ = this.update_in(cx, |view, window, cx| {
                        view.log_state.fail_command(message.clone());
                        window.push_notification(
                            Notification::error(message).title(tr("app.task.start_failed")),
                            cx,
                        );
                        view.replay_deferred_task_terminals(window, cx);
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
                view.replay_deferred_task_terminals(window, cx);
                match result {
                    Ok(_) => {
                        window.push_notification(
                            Notification::info(tr("app.task.stop_requested"))
                                .title(tr("app.common.stopping")),
                            cx,
                        );
                    }
                    Err(error) => {
                        window.push_notification(
                            Notification::error(
                                rust_i18n::t!("app.task.cancel_failed", error = error).to_string(),
                            )
                            .title(tr("app.task.cannot_stop")),
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

    fn defer_task_terminal(&mut self, snapshot: TaskSnapshot) {
        if let Some(existing) = self
            .deferred_task_terminals
            .iter_mut()
            .find(|existing| existing.id == snapshot.id)
        {
            *existing = snapshot;
        } else {
            self.deferred_task_terminals.push(snapshot);
        }
    }

    fn replay_deferred_task_terminals(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let snapshots = std::mem::take(&mut self.deferred_task_terminals);
        for snapshot in snapshots {
            self.on_task_terminal(snapshot, window, cx);
        }
    }

    fn on_task_terminal(
        &mut self,
        snapshot: TaskSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.restore_launcher(window, cx);
        if self.log_state.command_pending
            && self.log_state.active_id() == Some(snapshot.id)
            && !self.gui_task_ids.contains(&snapshot.id)
        {
            // A very short task can finish before its start call returns the
            // ID. Keep the terminal snapshot until the pending GUI command
            // resolves; it may belong to either that command or an Agent.
            self.defer_task_terminal(snapshot);
            return;
        }
        if !self.log_state.mark_terminal_notified(snapshot.id) {
            return;
        }
        if !self.gui_task_ids.remove(&snapshot.id) {
            self.refresh_after_external_task(&snapshot, window, cx);
            return;
        }
        let version = self.active_task_version.clone();
        match snapshot.status {
            TaskStatus::Completed if snapshot.kind == TaskKind::InstanceCreate => {
                let loader_source = self.create_copy_source.take();
                let is_loader_instance = loader_source.is_some();
                let (instance_id, name) = match snapshot.subject {
                    Some(TaskSubject::Instance { id, name }) => (Some(id), name),
                    _ => (None, tr("app.home.instance")),
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
                        Notification::success(
                            rust_i18n::t!("app.task.loader_instance_created", name = name)
                                .to_string(),
                        )
                        .title(tr("app.task.loader_install_complete")),
                        cx,
                    );
                } else {
                    window.push_notification(
                        Notification::success(
                            rust_i18n::t!("app.task.instance_created", name = name).to_string(),
                        )
                        .title(tr("app.task.instance_create_complete")),
                        cx,
                    );
                }
            }
            TaskStatus::Completed if snapshot.kind == TaskKind::ContentInstall => {
                let (instance_id, name) = match snapshot.subject {
                    Some(TaskSubject::Content {
                        instance_id, name, ..
                    }) => (Some(instance_id), name),
                    _ => (None, tr("app.task.content")),
                };
                if self.page == Page::Instance
                    && self.selected_instance.as_deref() == instance_id.as_deref()
                {
                    self.load_instance_content(window, cx);
                }
                window.push_notification(
                    Notification::success(
                        rust_i18n::t!("app.task.content_installed", name = name).to_string(),
                    )
                    .title(tr("app.task.content_install_complete")),
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
                    Notification::success(tr("app.task.install_complete_notice"))
                        .title(tr("app.task.install_complete")),
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
                            (Some(code), _) => {
                                rust_i18n::t!("app.logs.exit_code", code = code).to_string()
                            }
                            (_, Some(signal)) => {
                                rust_i18n::t!("app.logs.termination_signal", signal = signal)
                                    .to_string()
                            }
                            _ => tr("app.common.unknown_exit_status"),
                        })
                        .unwrap_or_else(|| tr("app.common.unknown_exit_status"));
                    window.push_notification(
                        Notification::warning(
                            rust_i18n::t!("app.task.game_exited_abnormally", detail = detail)
                                .to_string(),
                        )
                        .title(tr("app.task.game_abnormal_exit")),
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
                            .unwrap_or_else(|| tr("app.task.failed_check_logs")),
                    )
                    .title(tr("app.task.failed")),
                    cx,
                );
            }
            TaskStatus::Cancelled => {
                window.push_notification(
                    Notification::info(tr("app.task.cancelled_notice"))
                        .title(tr("app.task.cancelled")),
                    cx,
                );
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
            prompt: Some(tr("app.settings.choose_minecraft_dir").into()),
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
                            Notification::error(
                                rust_i18n::t!(
                                    "app.settings.directory_picker_failed",
                                    error = error
                                )
                                .to_string(),
                            )
                            .title(tr("app.settings.choose_directory_failed")),
                            cx,
                        );
                    });
                }
                Err(_) => {
                    let _ = this.update_in(cx, |_, window, cx| {
                        window.push_notification(
                            Notification::error(tr("app.settings.directory_picker_cancelled"))
                                .title(tr("app.settings.choose_directory_failed")),
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
            window.push_notification(
                Notification::error(error).title(tr("app.versions.read_failed")),
                cx,
            );
        } else {
            window.push_notification(
                Notification::success(tr("app.settings.game_dir_updated"))
                    .title(tr("app.settings.saved")),
                cx,
            );
        }
        cx.notify();
    }

    fn set_auto_java(&mut self, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.config.auto_install_java = enabled;
        self.save_config();
        window.push_notification(
            Notification::info(tr("app.settings.auto_java_saved")).title(tr("app.settings.saved")),
            cx,
        );
        cx.notify();
    }

    fn open_accounts_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let (accounts, heads, selected_id) = view
                .upgrade()
                .map(|view| {
                    let this = view.read(cx);
                    (
                        this.accounts.clone(),
                        this.account_heads.clone(),
                        this.config.selected_account_id.clone(),
                    )
                })
                .unwrap_or_default();

            let mut list = v_flex().w_full().gap_1().max_h_80().overflow_y_scrollbar();
            for account in accounts.iter() {
                let key = account_key(account);
                let username = account.username().to_owned();
                let is_selected = selected_id.as_deref() == Some(key.as_str());
                let subtitle = match account {
                    Account::Microsoft(_) => tr("app.account.kind_microsoft"),
                    Account::Authlib(account) => rust_i18n::t!(
                        "app.account.authlib_subtitle",
                        server = account
                            .server_name()
                            .map(str::to_owned)
                            .unwrap_or_else(|| tr("app.account.external_server"))
                    )
                    .to_string(),
                    Account::Offline(_) => tr("app.account.kind_offline"),
                };
                let head = heads.get(&account.uuid());
                let server_url = match account {
                    Account::Authlib(account) => Some(account.api_root().to_owned()),
                    _ => None,
                };
                let row_view = view.clone();
                let row_key = key.clone();
                let delete_view = view.clone();
                let delete_account = account.clone();
                list = list.child(
                    h_flex()
                        .id(format!("account-row-{key}"))
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
                                    this.select_account_key(&row_key, cx);
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
                                )
                                .when_some(server_url, |this, url| {
                                    this.child(
                                        div()
                                            .text_xs()
                                            .truncate()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(url),
                                    )
                                }),
                        )
                        .child(
                            Button::new(format!("account-delete-{key}"))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Trash)
                                .tooltip(tr("app.account.delete"))
                                .accessibility_label(tr("app.account.delete"))
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
                        .child(tr("app.account.not_added")),
                );
            }

            let login_view = view.clone();
            let offline_view = view.clone();
            let authlib_view = view.clone();
            dialog
                .title(tr("app.account.title"))
                .width(layout::rem_px(cx, layout::DIALOG_MEDIUM))
                .child(list)
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("accounts-close")
                                .ghost()
                                .label(tr("app.common.close"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("offline-add")
                                .outline()
                                .label(tr("app.account.add_offline"))
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
                            Button::new("authlib-login")
                                .outline()
                                .label(tr("app.authlib.login"))
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    if let Some(view) = authlib_view.upgrade() {
                                        view.update(cx, |this, cx| {
                                            this.open_authlib_login_dialog(window, cx)
                                        });
                                    }
                                }),
                        )
                        .child(
                            Button::new("microsoft-login")
                                .primary()
                                .label(tr("app.account.microsoft_login"))
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
                    Account::Microsoft(_) | Account::Authlib(_) => None,
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
            tr("app.account.edit_offline")
        } else {
            tr("app.account.add_offline")
        };
        let first_focus = Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if first_focus.replace(false) {
                input.read(cx).focus_handle(cx).focus(window, cx);
            }
            dialog
                .title(title.clone())
                .child(
                    v_flex().gap_3().child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_medium()
                                    .child(tr("app.account.player_name")),
                            )
                            .child(div().p_1().child(Input::new(&input).w_full())),
                    ),
                )
                .footer(
                    DialogFooter::new()
                        .child(
                            DialogClose::new()
                                .trigger(|button| button.label(tr("app.common.cancel")).outline()),
                        )
                        .child(
                            DialogAction::new().child(
                                Button::new("save-account")
                                    .primary()
                                    .label(tr("app.account.save_and_switch")),
                            ),
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

    fn open_authlib_login_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.authlib_login = None;
        self.authlib_login_cancel = None;
        self.authlib_custom_server = false;
        self.authlib_server_select.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::default().row(0)), window, cx);
        });
        for input in [
            &self.authlib_server_input,
            &self.authlib_username_input,
            &self.authlib_password_input,
        ] {
            input.update(cx, |state, cx| state.set_value("", window, cx));
        }

        let view = cx.entity().downgrade();
        let server_select = self.authlib_server_select.clone();
        let server_input = self.authlib_server_input.clone();
        let username_input = self.authlib_username_input.clone();
        let password_input = self.authlib_password_input.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            let (state, custom, selected_server) = view
                .upgrade()
                .map(|view| {
                    let this = view.read(cx);
                    let selected_server = this
                        .authlib_server_select
                        .read(cx)
                        .selected_index(cx)
                        .and_then(|index| AUTHLIB_SERVER_PRESETS.get(index.row))
                        .map(|preset| preset.api_root.to_owned())
                        .unwrap_or_default();
                    (
                        this.authlib_login.clone(),
                        this.authlib_custom_server,
                        selected_server,
                    )
                })
                .unwrap_or((None, false, String::new()));
            let requesting = matches!(state, Some(AuthlibLoginUi::Requesting));
            let selecting = match state.as_ref() {
                Some(AuthlibLoginUi::SelectingProfile { login, profiles }) => {
                    Some((login.clone(), profiles.clone()))
                }
                _ => None,
            };
            let failed = match state.as_ref() {
                Some(AuthlibLoginUi::Failed { message }) => Some(message.clone()),
                _ => None,
            };

            let content = if let Some((login, profiles)) = selecting.as_ref() {
                let mut list = v_flex().gap_2().child(
                    div().text_sm().font_medium().child(
                        rust_i18n::t!(
                            "app.authlib.choose_profile",
                            server = login.server().display_name()
                        )
                        .to_string(),
                    ),
                );
                for profile in profiles {
                    let profile_view = view.clone();
                    let profile_clone = profile.clone();
                    list = list.child(
                        Button::new(format!("authlib-profile-{}", profile.id()))
                            .outline()
                            .w_full()
                            .label(profile.name().to_owned())
                            .on_click(move |_, window, cx| {
                                if let Some(view) = profile_view.upgrade() {
                                    view.update(cx, |this, cx| {
                                        this.complete_authlib_login_profile(
                                            profile_clone.clone(),
                                            window,
                                            cx,
                                        )
                                    });
                                }
                            }),
                    );
                }
                list.into_any_element()
            } else {
                let mut form = v_flex()
                    .gap_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(field_label(tr("app.authlib.server")))
                            .child(
                                Select::new(&server_select)
                                    .accessibility_label(tr("app.authlib.server"))
                                    .menu_width(layout::rem_px(cx, layout::SELECT_MENU_WIDTH))
                                    .menu_max_h(layout::rem_px(cx, layout::SELECT_MENU_WIDTH))
                                    .cleanable(false)
                                    .w_full(),
                            ),
                    )
                    .child(if custom {
                        v_flex()
                            .gap_1()
                            .child(field_label(tr("app.authlib.server_address")))
                            .child(Input::new(&server_input).w_full())
                            .child(help(tr("app.authlib.server_address_help"), cx))
                            .into_any_element()
                    } else {
                        v_flex()
                            .gap_1()
                            .child(help(
                                rust_i18n::t!("app.authlib.api_address", address = selected_server)
                                    .to_string(),
                                cx,
                            ))
                            .into_any_element()
                    })
                    .child(
                        v_flex()
                            .gap_1()
                            .child(field_label(tr("app.authlib.email_or_username")))
                            .child(Input::new(&username_input).w_full()),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(field_label(tr("app.authlib.password")))
                            .child(Input::new(&password_input).mask_toggle().w_full())
                            .child(help(tr("app.authlib.password_help"), cx)),
                    );
                if let Some(message) = failed {
                    form = form.child(div().text_xs().text_color(cx.theme().danger).child(message));
                }
                if requesting {
                    form = form.child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Spinner::new()
                                    .with_size(Size::Small)
                                    .color(cx.theme().primary),
                            )
                            .child(div().text_sm().child(tr("app.authlib.logging_in"))),
                    );
                }
                form.into_any_element()
            };

            let mut footer = DialogFooter::new().child(
                DialogClose::new()
                    .trigger(|button| button.label(tr("app.common.cancel")).outline()),
            );
            if selecting.is_none() {
                footer = footer.child(
                    DialogAction::new().child(
                        Button::new("authlib-submit")
                            .primary()
                            .label(tr("app.authlib.login_button")),
                    ),
                );
            }

            let close_view = view.clone();
            dialog
                .title(tr("app.authlib.login"))
                .width(layout::rem_px(cx, layout::DIALOG_MEDIUM))
                .child(content)
                .footer(footer)
                .on_close(move |_, window, cx| {
                    if let Some(view) = close_view.upgrade() {
                        view.update(cx, |this, cx| this.clear_authlib_login(window, cx));
                    }
                })
                .on_ok({
                    let view = view.clone();
                    move |_, window, cx| {
                        let Some(view) = view.upgrade() else {
                            return true;
                        };
                        view.update(cx, |this, cx| {
                            if matches!(
                                this.authlib_login,
                                Some(AuthlibLoginUi::SelectingProfile { .. })
                            ) || matches!(this.authlib_login, Some(AuthlibLoginUi::Requesting))
                            {
                                return false;
                            }
                            this.begin_authlib_login(window, cx)
                        })
                    }
                })
        });
    }

    fn begin_authlib_login(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if matches!(self.authlib_login, Some(AuthlibLoginUi::Requesting)) {
            return false;
        }
        let server = if self.authlib_custom_server {
            self.authlib_server_input.read(cx).value().trim().to_owned()
        } else {
            self.authlib_server_select
                .read(cx)
                .selected_index(cx)
                .and_then(|index| AUTHLIB_SERVER_PRESETS.get(index.row))
                .map(|preset| preset.api_root.to_owned())
                .unwrap_or_default()
        };
        if server.is_empty() {
            window.push_notification(
                Notification::error(tr("app.authlib.server_required"))
                    .title(tr("app.authlib.login_failed")),
                cx,
            );
            return false;
        }
        let username = self
            .authlib_username_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        let password = self.authlib_password_input.read(cx).value().to_string();
        if username.is_empty() {
            window.push_notification(
                Notification::error(tr("app.authlib.username_required"))
                    .title(tr("app.authlib.login_failed")),
                cx,
            );
            return false;
        }
        if password.is_empty() {
            window.push_notification(
                Notification::error(tr("app.authlib.password_required"))
                    .title(tr("app.authlib.login_failed")),
                cx,
            );
            return false;
        }

        self.authlib_login = Some(AuthlibLoginUi::Requesting);
        let cancellation = CancellationToken::new();
        self.authlib_login_cancel = Some(cancellation.clone());
        self.spawn_authlib_login(server, username, password, cancellation, window, cx);
        cx.notify();
        // Keep the dialog open while the request runs. Closing it here would
        // cancel the login task and hide every error behind a disappearing
        // dialog.
        false
    }

    fn spawn_authlib_login(
        &mut self,
        server: String,
        username: String,
        password: String,
        cancellation: CancellationToken,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let service = self.services.authlib_accounts.clone();
        let runtime = self.services.runtime.handle().clone();
        self._authlib_login_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = runtime
                .spawn(async move {
                    service
                        .authenticate(&server, &username, &password, &cancellation)
                        .await
                })
                .await;
            let _ = this.update_in(cx, move |view, window, cx| {
                if view.authlib_login.is_none() {
                    return;
                }
                match result {
                    Ok(Ok(login)) => {
                        if let Some(profile) = login.single_profile().cloned() {
                            view.complete_authlib_login_with_profile(login, profile, window, cx);
                        } else if login.available_profiles().is_empty() {
                            view.fail_authlib_login(tr("app.authlib.no_profiles"), cx);
                        } else {
                            let profiles = login.available_profiles().to_vec();
                            view.authlib_login = Some(AuthlibLoginUi::SelectingProfile {
                                login: Box::new(login),
                                profiles,
                            });
                            view.authlib_login_cancel = None;
                            cx.notify();
                        }
                    }
                    Ok(Err(error)) => {
                        view.fail_authlib_login(authlib_login_error_message(&error), cx)
                    }
                    Err(error) => view.fail_authlib_login(
                        rust_i18n::t!("app.common.background_task_failed", error = error)
                            .to_string(),
                        cx,
                    ),
                }
            });
        }));
    }

    fn complete_authlib_login_profile(
        &mut self,
        profile: AuthlibProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(AuthlibLoginUi::SelectingProfile { login, .. }) = self.authlib_login.clone()
        else {
            return;
        };
        self.complete_authlib_login_with_profile(*login, profile, window, cx);
    }

    fn complete_authlib_login_with_profile(
        &mut self,
        login: AuthlibLogin,
        profile: AuthlibProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let account_id = self
            .accounts
            .iter()
            .find_map(|account| match account {
                Account::Authlib(account)
                    if account.api_root() == login.server().api_root()
                        && account.uuid() == profile.id() =>
                {
                    Some(account.id())
                }
                _ => None,
            })
            .unwrap_or_else(Uuid::new_v4);
        let cancellation = self.authlib_login_cancel.clone().unwrap_or_default();
        self.authlib_login_cancel = Some(cancellation.clone());
        self.authlib_login = Some(AuthlibLoginUi::Requesting);

        let service = self.services.authlib_accounts.clone();
        let runtime = self.services.runtime.handle().clone();
        self._authlib_login_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = runtime
                .spawn(async move {
                    service
                        .complete_login_with_id(login, &profile, account_id, &cancellation)
                        .await
                })
                .await;
            let _ = this.update_in(cx, move |view, window, cx| {
                if view.authlib_login.is_none() {
                    return;
                }
                match result {
                    Ok(Ok(account)) => {
                        view.authlib_login = None;
                        view.authlib_login_cancel = None;
                        view.add_authlib_account(account, window, cx);
                        window.close_dialog(cx);
                        window.push_notification(
                            Notification::success(tr("app.authlib.logged_in"))
                                .title(tr("app.account.title")),
                            cx,
                        );
                    }
                    Ok(Err(error)) => {
                        view.fail_authlib_login(authlib_login_error_message(&error), cx)
                    }
                    Err(error) => view.fail_authlib_login(
                        rust_i18n::t!("app.common.background_task_failed", error = error)
                            .to_string(),
                        cx,
                    ),
                }
                cx.notify();
            });
        }));
    }

    fn fail_authlib_login(&mut self, message: String, cx: &mut Context<Self>) {
        self.authlib_login = Some(AuthlibLoginUi::Failed { message });
        self.authlib_login_cancel = None;
        cx.notify();
    }

    fn clear_authlib_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.authlib_login = None;
        if let Some(cancellation) = self.authlib_login_cancel.take() {
            cancellation.cancel();
        }
        self.authlib_password_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    fn add_authlib_account(
        &mut self,
        account: AuthlibAccount,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = account.id();
        self.accounts.retain(
            |existing| !matches!(existing, Account::Authlib(existing) if existing.id() == id),
        );
        self.accounts.push(Account::Authlib(account.clone()));
        self.config.selected_account = Some(account.username().to_owned());
        self.config.selected_account_kind = StoredAccountKind::Authlib;
        self.config.selected_account_id = Some(format!("authlib:{id}"));
        self.save_config();
        if self.page == Page::Skin {
            self.ensure_skin_page(cx);
        }
        cx.notify();
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
                    let message = rust_i18n::t!("app.common.background_task_failed", error = error)
                        .to_string();
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
                                    Notification::success(tr("app.account.microsoft_logged_in"))
                                        .title(tr("app.account.title")),
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
                            let message =
                                rust_i18n::t!("app.common.background_task_failed", error = error)
                                    .to_string();
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
                    let message = rust_i18n::t!("app.common.background_task_failed", error = error)
                        .to_string();
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
                            .child(div().text_sm().child(tr("app.account.fetching_code"))),
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
                                .child(
                                    rust_i18n::t!(
                                        "app.account.open_and_enter_code",
                                        site = verification_label
                                    )
                                    .to_string(),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_medium()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(tr("app.account.login_code")),
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
                                                    tr("app.account.copied")
                                                } else {
                                                    tr("app.account.copy_code")
                                                })
                                                .accessibility_label(if code_copied {
                                                    tr("app.account.copied")
                                                } else {
                                                    tr("app.account.copy_code")
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
                                .cursor_pointer()
                                .icon(IconName::ExternalLink)
                                .label(tr("app.account.open_browser"))
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
                                        .child(tr("app.account.waiting_for_login")),
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
                            .child(div().text_sm().child(tr("app.account.fetching_profile"))),
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
                            .label(tr("app.common.close"))
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
                            .label(tr("app.common.retry"))
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
                        .label(tr("app.common.cancel"))
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
                .title(tr("app.account.microsoft_login"))
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
                Account::Offline(_) | Account::Authlib(_) => None,
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
        self.config.selected_account_id = Some(format!("microsoft:{}", account.uuid()));
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
        let kind = account.kind();
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let clicked_account = account.clone();
            let confirm_view = view.clone();
            dialog
                .title(tr("app.account.delete"))
                .width(layout::rem_px(cx, layout::DIALOG_SMALL))
                .child(
                    v_flex()
                        .gap_2()
                        .child(div().text_sm().child(
                            rust_i18n::t!("app.account.delete_confirm", name = name).to_string(),
                        ))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(match kind {
                                    AccountKind::Microsoft => {
                                        tr("app.account.delete_microsoft_notice")
                                    }
                                    AccountKind::Authlib => tr("app.account.delete_authlib_notice"),
                                    AccountKind::Offline => tr("app.account.delete_offline_notice"),
                                }),
                        ),
                )
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("cancel-delete-account")
                                .ghost()
                                .label(tr("app.common.cancel"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("confirm-delete-account")
                                .danger()
                                .label(tr("app.common.delete"))
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
        match &account {
            Account::Offline(_) => self.finish_delete_account(account, window, cx),
            Account::Microsoft(profile) => {
                let profile = profile.clone();
                let service = self.services.microsoft_accounts.clone();
                let runtime = self.services.runtime.handle().clone();
                self._account_task = Some(cx.spawn_in(window, async move |this, cx| {
                    let result = runtime
                        .spawn({
                            let service = service.clone();
                            let profile = profile.clone();
                            async move { service.remove_credentials(&profile).await }
                        })
                        .await;
                    let _ = this.update_in(cx, move |view, window, cx| {
                        match result {
                            Ok(Ok(())) => {
                                view.finish_delete_account(Account::Microsoft(profile), window, cx)
                            }
                            Ok(Err(error)) => {
                                window.push_notification(
                                    Notification::error(
                                        rust_i18n::t!(
                                            "app.account.credential_delete_failed",
                                            error = error
                                        )
                                        .to_string(),
                                    )
                                    .title(tr("app.account.delete_failed")),
                                    cx,
                                );
                            }
                            Err(error) => {
                                window.push_notification(
                                    Notification::error(
                                        rust_i18n::t!(
                                            "app.common.background_task_failed",
                                            error = error
                                        )
                                        .to_string(),
                                    )
                                    .title(tr("app.account.delete_failed")),
                                    cx,
                                );
                            }
                        }
                        cx.notify();
                    });
                }));
            }
            Account::Authlib(profile) => {
                let profile = profile.clone();
                let service = self.services.authlib_accounts.clone();
                let runtime = self.services.runtime.handle().clone();
                self._account_task = Some(cx.spawn_in(window, async move |this, cx| {
                    let result = runtime
                        .spawn({
                            let service = service.clone();
                            let profile = profile.clone();
                            async move { service.remove_credentials(&profile).await }
                        })
                        .await;
                    let _ = this.update_in(cx, move |view, window, cx| {
                        match result {
                            Ok(Ok(())) => {
                                view.finish_delete_account(Account::Authlib(profile), window, cx)
                            }
                            Ok(Err(error)) => {
                                window.push_notification(
                                    Notification::error(
                                        rust_i18n::t!(
                                            "app.account.credential_delete_failed",
                                            error = error
                                        )
                                        .to_string(),
                                    )
                                    .title(tr("app.account.delete_failed")),
                                    cx,
                                );
                            }
                            Err(error) => {
                                window.push_notification(
                                    Notification::error(
                                        rust_i18n::t!(
                                            "app.common.background_task_failed",
                                            error = error
                                        )
                                        .to_string(),
                                    )
                                    .title(tr("app.account.delete_failed")),
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
            self.config.selected_account_id = self.accounts.first().map(account_key);
        }
        self.save_config();
        if self.page == Page::Skin {
            self.skin_editor.draft = None;
            self.skin_apply = ApplyState::Idle;
            self.ensure_skin_page(cx);
        }
        window.push_notification(
            Notification::success(tr("app.account.deleted")).title(tr("app.account.title")),
            cx,
        );
        cx.notify();
    }

    fn save_account(&mut self, value: String, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let username = value.trim().to_owned();
        if username.is_empty() {
            window.push_notification(
                Notification::error(tr("app.account.username_required"))
                    .title(tr("app.account.save_failed")),
                cx,
            );
            return false;
        }
        let account = match OfflineAccount::new(username.clone()) {
            Ok(account) => account,
            Err(error) => {
                window.push_notification(
                    Notification::error(error.to_string()).title(tr("app.account.save_failed")),
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
        let uuid = account.uuid();
        self.accounts.push(Account::Offline(account));
        self.config.selected_account = Some(username);
        self.config.selected_account_kind = StoredAccountKind::Offline;
        self.config.selected_account_id = Some(format!("offline:{uuid}"));
        self.save_config();
        window.push_notification(
            Notification::success(tr("app.account.offline_saved")).title(tr("app.account.title")),
            cx,
        );
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
        let target = display_path(&self.config.minecraft_root);
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            dialog
                .title(tr("app.install.dialog_title"))
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
                                .child(div().text_xs().child(tr("app.install.install_to")))
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
                                .child(tr("app.install.isolation_notice")),
                        ),
                )
                .footer(
                    DialogFooter::new()
                        .child(
                            DialogClose::new()
                                .trigger(|button| button.label(tr("app.common.cancel")).outline()),
                        )
                        .child(
                            DialogAction::new().child(
                                Button::new("install-version")
                                    .primary()
                                    .label(tr("app.common.install")),
                            ),
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
            window.push_notification(Notification::info(tr("app.logs.nothing_to_copy")), cx);
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        window.push_notification(
            Notification::success(tr("app.logs.copied")).title(tr("app.account.copied")),
            cx,
        );
    }

    fn export_logs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.log_state.active_id() else {
            window.push_notification(Notification::info(tr("app.logs.no_task_logs")), cx);
            return;
        };
        let directory = self.services.config_store.logs_dir();
        if let Err(error) = std::fs::create_dir_all(&directory) {
            window.push_notification(
                Notification::error(
                    rust_i18n::t!("app.logs.create_dir_failed", error = error).to_string(),
                )
                .title(tr("app.logs.export_failed")),
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
                Notification::success(
                    rust_i18n::t!("app.logs.exported", path = display_path(&path)).to_string(),
                )
                .title(tr("app.logs.exported_title")),
                cx,
            ),
            Err(error) => window.push_notification(
                Notification::error(
                    rust_i18n::t!("app.logs.write_failed", error = error).to_string(),
                )
                .title(tr("app.logs.export_failed")),
                cx,
            ),
        }
    }

    fn open_logs_dir(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let directory = self.services.config_store.logs_dir();
        let _ = std::fs::create_dir_all(&directory);
        if let Err(error) = open_path(&directory) {
            window.push_notification(
                Notification::error(
                    rust_i18n::t!("app.logs.open_dir_failed", error = error).to_string(),
                )
                .title(tr("app.common.open_failed")),
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
                Notification::warning(tr("app.logs.stop_first")).title(tr("app.logs.cannot_clear")),
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
            tr("app.versions.delete_isolated_description")
        } else {
            tr("app.versions.delete_shared_description")
        };
        let view = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .confirm()
                .title(rust_i18n::t!("app.versions.delete_title", version = version_id).to_string())
                .description(description.clone())
                .ok_text(tr("app.common.delete"))
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
                Notification::warning(tr("app.versions.wait_for_task"))
                    .title(tr("app.versions.cannot_delete")),
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
                        Notification::success(
                            rust_i18n::t!("app.versions.deleted", version = display).to_string(),
                        )
                        .title(tr("app.versions.deleted_title")),
                        cx,
                    );
                }
                Ok(Err(error)) => {
                    window.push_notification(
                        Notification::error(error.to_string())
                            .title(tr("app.versions.delete_failed")),
                        cx,
                    );
                }
                Err(error) => {
                    window.push_notification(
                        Notification::error(
                            rust_i18n::t!("app.common.background_task_failed", error = error)
                                .to_string(),
                        )
                        .title(tr("app.versions.delete_failed")),
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
                Notification::error(
                    rust_i18n::t!("app.instance.open_folder_failed", error = error).to_string(),
                )
                .title(tr("app.common.open_failed")),
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
                Notification::error(
                    rust_i18n::t!("app.instance.open_game_folder_failed", error = error)
                        .to_string(),
                )
                .title(tr("app.common.open_failed")),
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
                .title(rust_i18n::t!("app.versions.isolation_title", name = name).to_string())
                .description(tr("app.versions.isolation_description"))
                .ok_text(tr("app.versions.enable_isolation_confirm"))
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
                Notification::warning(tr("app.versions.wait_for_task"))
                    .title(tr("app.versions.cannot_enable_isolation")),
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
                            Notification::success(
                                rust_i18n::t!("app.versions.isolation_enabled", version = display)
                                    .to_string(),
                            )
                            .title(tr("app.versions.isolation_enabled_title")),
                            cx,
                        );
                    }
                    Ok(Err(error)) => {
                        window.push_notification(
                            Notification::error(error.to_string())
                                .title(tr("app.versions.isolation_failed")),
                            cx,
                        );
                    }
                    Err(error) => {
                        window.push_notification(
                            Notification::error(
                                rust_i18n::t!("app.common.background_task_failed", error = error)
                                    .to_string(),
                            )
                            .title(tr("app.versions.isolation_failed")),
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
                Notification::warning(tr("app.instance.content_busy"))
                    .title(tr("app.instance.cannot_open")),
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
                Notification::error(tr("app.instance.missing_notice"))
                    .title(tr("app.instance.cannot_open")),
                cx,
            );
            return;
        }
        self.selected_instance = Some(version_id);
        self.page = Page::Instance;
        self.content_tab = ContentTab::Mods;
        self.content_library = None;
        self.content_inspection = None;
        self.content_icons.clear();
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
                .title(tr("app.instance.rename"))
                .width(layout::rem_px(cx, layout::DIALOG_SMALL))
                .child(
                    v_flex().gap_3().child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_medium()
                                    .child(tr("app.create_instance.display_name")),
                            )
                            .child(div().p_1().child(Input::new(&input).w_full()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(tr("app.instance.rename_hint")),
                            ),
                    ),
                )
                .footer(
                    DialogFooter::new()
                        .child(
                            DialogClose::new()
                                .trigger(|button| button.label(tr("app.common.cancel")).outline()),
                        )
                        .child(
                            DialogAction::new().child(
                                Button::new("save-instance-name")
                                    .primary()
                                    .label(tr("app.common.save")),
                            ),
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
                Notification::error(tr("app.instance.missing_notice"))
                    .title(tr("app.instance.rename_failed")),
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
                    .title(tr("app.instance.renamed")),
                    cx,
                );
                true
            }
            Err(error) => {
                window.push_notification(
                    Notification::error(error.to_string()).title(tr("app.instance.rename_failed")),
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
                .title(tr("app.instance.launch_settings"))
                .width(layout::rem_px(cx, layout::DIALOG_MEDIUM))
                .child(
                    v_flex()
                        .gap_4()
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_medium()
                                        .child(tr("app.instance.max_memory")),
                                )
                                .child(div().child(Input::new(&memory_input).w_full()))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(tr("app.instance.max_memory_hint")),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_medium()
                                        .child(tr("app.instance.window_size")),
                                )
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
                                        .child(tr("app.instance.window_size_hint")),
                                ),
                        ),
                )
                .footer(
                    DialogFooter::new()
                        .child(
                            DialogClose::new()
                                .trigger(|button| button.label(tr("app.common.cancel")).outline()),
                        )
                        .child(
                            DialogAction::new().child(
                                Button::new("save-launch-settings")
                                    .primary()
                                    .label(tr("app.common.save")),
                            ),
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
                Notification::error(tr("app.instance.numeric_required"))
                    .title(tr("app.instance.save_launch_failed")),
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
                Notification::error(tr("app.instance.missing_notice"))
                    .title(tr("app.instance.save_launch_failed")),
                cx,
            );
            return false;
        };
        let layout = version.instance_layout(self.config.minecraft_root.clone());
        match set_instance_launch_settings(&layout, memory, width, height) {
            Ok(()) => {
                self.reload_versions(window, cx);
                window.push_notification(
                    Notification::info(tr("app.instance.launch_settings_updated"))
                        .title(tr("app.instance.launch_settings_saved")),
                    cx,
                );
                true
            }
            Err(error) => {
                window.push_notification(
                    Notification::error(error.to_string())
                        .title(tr("app.instance.save_launch_failed")),
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
                return dialog.title(tr("app.java.select"));
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
                            .child(div().text_sm().font_medium().child(tr("app.java.auto")))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(tr("app.java.auto_description")),
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
                                            display_path(&runtime.executable)
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
                        .child(tr("app.java.instance_only_notice")),
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
                            .child(div().text_sm().child(tr("app.java.probing"))),
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
                        .child(
                            div()
                                .text_sm()
                                .font_medium()
                                .child(tr("app.java.custom_executable")),
                        )
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
                                            tr("app.java.detecting")
                                        } else {
                                            tr("app.java.use_path")
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
                                .child(tr("app.java.path_hint")),
                        ),
                );

            let retry_view = view.downgrade();
            dialog
                .title(tr("app.java.select"))
                .width(layout::rem_px(cx, layout::DIALOG_LARGE))
                .child(body)
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("java-dialog-close")
                                .ghost()
                                .label(tr("app.common.close"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("java-dialog-retry")
                                .outline()
                                .label(tr("app.java.redetect"))
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
                                tr("app.java.not_found")
                            } else {
                                rust_i18n::t!("app.java.none_available", count = failure_count)
                                    .to_string()
                            });
                        }
                    }
                    Err(error) => {
                        this.java_dialog_error =
                            Some(rust_i18n::t!("app.java.probe_failed", error = error).to_string());
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
            let result = result.unwrap_or_else(|error| {
                Err(rust_i18n::t!("app.java.probe_failed", error = error).to_string())
            });
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
                            Notification::error(error).title(tr("app.java.cannot_use")),
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
                Notification::error(tr("app.instance.missing_notice"))
                    .title(tr("app.java.save_failed")),
                cx,
            );
            return false;
        };
        let layout = version.instance_layout(self.config.minecraft_root.clone());
        match set_instance_java(&layout, java.clone()) {
            Ok(()) => {
                self.java_dialog_target = None;
                self.reload_versions(window, cx);
                let label = java.map_or_else(|| tr("app.java.auto"), |path| display_path(&path));
                window
                    .push_notification(Notification::info(label).title(tr("app.java.updated")), cx);
                true
            }
            Err(error) => {
                window.push_notification(
                    Notification::error(error.to_string()).title(tr("app.java.save_failed")),
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
                Notification::error(tr("app.instance.missing_or_moved"))
                    .title(tr("app.loader.cannot_add")),
                cx,
            );
            return;
        };
        if let Some(loader) = version.loader.as_ref() {
            window.push_notification(
                Notification::warning(
                    rust_i18n::t!(
                        "app.loader.already_installed",
                        kind = loader.kind.label(),
                        version = loader.version
                    )
                    .to_string(),
                )
                .title(tr("app.loader.cannot_add")),
                cx,
            );
            return;
        }
        if !version.manifest_present {
            window.push_notification(
                Notification::warning(tr("app.instance.loader_legacy"))
                    .title(tr("app.loader.cannot_add")),
                cx,
            );
            return;
        }
        if let Some(error) = version.manifest_error.as_ref() {
            window.push_notification(
                Notification::error(error.clone()).title(tr("app.loader.cannot_add")),
                cx,
            );
            return;
        }
        if !self.can_start_operation() {
            window.push_notification(
                Notification::warning(tr("app.common.operation_busy"))
                    .title(tr("app.loader.cannot_add")),
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
                .unwrap_or_else(|| tr("app.home.instance"));
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
            self.create_loader_error = Some(tr("app.create_instance.version_required"));
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
                        view.create_loader_error = Some(
                            rust_i18n::t!("app.common.background_task_failed", error = error)
                                .to_string(),
                        );
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
                Notification::warning(tr("app.common.operation_busy"))
                    .title(tr("app.create_instance.cannot_create")),
                cx,
            );
            return;
        }
        if self.create_copy_source.is_some() && self.create_loader.is_none() {
            window.push_notification(
                Notification::error(tr("app.create_instance.loader_required"))
                    .title(tr("app.loader.cannot_add")),
                cx,
            );
            return;
        }
        let base = self.create_base_input.read(cx).value().trim().to_owned();
        if base.is_empty() {
            window.push_notification(
                Notification::error(tr("app.create_instance.version_required"))
                    .title(tr("app.create_instance.cannot_create")),
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
                Notification::error(tr("app.create_instance.id_required"))
                    .title(tr("app.create_instance.cannot_create")),
                cx,
            );
            return;
        }
        let name = self.create_name_input.read(cx).value().trim().to_owned();
        let loader = match self.create_loader {
            Some(kind) => {
                let Some(version) = self.create_loader_version.clone() else {
                    window.push_notification(
                        Notification::error(tr("app.create_instance.loader_version_required"))
                            .title(tr("app.create_instance.cannot_create")),
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
            Notification::info(
                rust_i18n::t!("app.create_instance.creating", id = instance_id).to_string(),
            )
            .title(tr("app.create_instance.started")),
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
                Notification::error(tr("app.instance.missing_notice"))
                    .title(tr("app.content_browser.cannot_open")),
                cx,
            );
            return;
        };
        self.cancel_content_browser_request();
        self.selected_instance = Some(version.id.as_str().to_owned());
        self.content_tab = ContentTab::from_kind(kind);
        self.page = Page::ContentBrowser;
        self.content_browser = ContentBrowserState {
            kind: Some(kind),
            shader_loader: self
                .content_inspection
                .as_ref()
                .and_then(|inspection| inspection.shader_loader),
            ..ContentBrowserState::default()
        };
        self.remote_sort = RemoteSort::Relevance;
        self.remote_query.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        self.load_remote_search(true, window, cx);
    }

    fn switch_content_browser_kind(
        &mut self,
        kind: ContentKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.content_browser.preflighting || self.content_browser.kind == Some(kind) {
            return;
        }
        self.cancel_content_browser_request();
        self.content_tab = ContentTab::from_kind(kind);
        self.content_browser.kind = Some(kind);
        self.content_browser.project = None;
        self.content_browser.gallery = None;
        self.content_browser.projects.clear();
        self.content_browser.total_hits = 0;
        self.content_browser.versions.clear();
        self.content_browser.versions_loading = false;
        self.content_browser.search_error = None;
        self.content_browser.versions_error = None;
        self.content_browser.recommended = None;
        self.content_browser.preflighting = false;
        self.load_remote_search(true, window, cx);
    }

    fn open_shader_loader_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_content_browser(ContentKind::Mod, window, cx);
        self.remote_query.update(cx, |state, cx| {
            state.set_value("Iris", window, cx);
        });
        self.load_remote_search(true, window, cx);
    }

    fn back_to_instance_detail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_content_browser_request();
        self.content_browser.gallery = None;
        self.page = Page::Instance;
        self.load_instance_content(window, cx);
    }

    fn cancel_content_browser_request(&mut self) {
        if let Some(cancellation) = self.content_browser_cancel.take() {
            cancellation.cancel();
        }
        self.cancel_content_icons();
        self.cancel_content_previews();
        self.content_browser_request = self.content_browser_request.wrapping_add(1);
    }

    fn cancel_content_icons(&mut self) {
        if let Some(cancellation) = self.content_browser_icons_cancel.take() {
            cancellation.cancel();
        }
        self._content_icons_task = None;
    }

    fn cancel_content_previews(&mut self) {
        if let Some(cancellation) = self.content_browser_previews_cancel.take() {
            cancellation.cancel();
        }
        self._content_previews_task = None;
    }

    fn load_project_icons(
        &mut self,
        projects: &[RemoteProject],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pending = projects
            .iter()
            .filter_map(|project| {
                let url = project.icon_url.as_deref()?;
                (!self.content_browser.project_icons.contains_key(&project.id))
                    .then(|| (project.id.clone(), url.to_owned()))
            })
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return;
        }

        self.cancel_content_icons();
        let cancellation = CancellationToken::new();
        self.content_browser_icons_cancel = Some(cancellation.clone());
        let cache = self.services.remote_icons.clone();
        let runtime = self.services.runtime.handle().clone();
        self._content_icons_task = Some(cx.spawn_in(window, async move |this, cx| {
            let results = runtime
                .spawn(async move {
                    let mut tasks = tokio::task::JoinSet::new();
                    for (id, url) in pending {
                        let cache = cache.clone();
                        let cancellation = cancellation.clone();
                        tasks.spawn(async move {
                            let path = cache.ensure(&url, &cancellation).await.ok();
                            (id, path)
                        });
                    }
                    let mut results = Vec::new();
                    while let Some(joined) = tasks.join_next().await {
                        if let Ok(result) = joined {
                            results.push(result);
                        }
                    }
                    results
                })
                .await;
            let _ = this.update_in(cx, |view, _, cx| {
                if let Ok(results) = results {
                    for (id, path) in results {
                        view.content_browser.project_icons.insert(id, path);
                    }
                    cx.notify();
                }
            });
        }));
    }

    fn load_project_previews(
        &mut self,
        projects: &[RemoteProject],
        include_gallery: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut pending = Vec::new();
        let mut seen = HashSet::new();
        for project in projects {
            if !matches!(
                project.kind(),
                Some(ContentKind::ResourcePack | ContentKind::ShaderPack)
            ) {
                continue;
            }
            let urls = if include_gallery {
                project.gallery_previews().into_iter().take(6).collect()
            } else {
                project.preview_url().into_iter().collect::<Vec<_>>()
            };
            for url in urls {
                if seen.insert(url.to_owned())
                    && !self.content_browser.project_previews.contains_key(url)
                {
                    pending.push(url.to_owned());
                }
            }
        }
        if pending.is_empty() {
            return;
        }

        self.cancel_content_previews();
        let cancellation = CancellationToken::new();
        self.content_browser_previews_cancel = Some(cancellation.clone());
        let cache = self.services.remote_gallery.clone();
        let runtime = self.services.runtime.handle().clone();
        self._content_previews_task = Some(cx.spawn_in(window, async move |this, cx| {
            let results = runtime
                .spawn(async move {
                    let mut tasks = tokio::task::JoinSet::new();
                    for url in pending {
                        let cache = cache.clone();
                        let cancellation = cancellation.clone();
                        tasks.spawn(async move {
                            let path = cache.ensure(&url, &cancellation).await.ok();
                            (url, path)
                        });
                    }
                    let mut results = Vec::new();
                    while let Some(joined) = tasks.join_next().await {
                        if let Ok(result) = joined {
                            results.push(result);
                        }
                    }
                    results
                })
                .await;
            let _ = this.update_in(cx, |view, _, cx| {
                if let Ok(results) = results {
                    for (url, path) in results {
                        view.content_browser.project_previews.insert(url, path);
                    }
                    cx.notify();
                }
            });
        }));
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
            let _ = this.update_in(cx, |view, window, cx| {
                if view.content_browser_request != request {
                    return;
                }
                view.content_browser.search_loading = false;
                match result {
                    Ok(Ok(page)) => {
                        if reset {
                            view.content_browser.projects = page.projects;
                            view.content_browser
                                .project_icons
                                .retain(|_, path| path.is_some());
                            view.content_browser
                                .project_previews
                                .retain(|_, path| path.is_some());
                        } else {
                            view.content_browser.projects.extend(page.projects);
                        }
                        view.content_browser.total_hits = page.total_hits;
                        view.content_browser.search_error = None;
                        let projects = view.content_browser.projects.clone();
                        view.load_project_icons(&projects, window, cx);
                        view.load_project_previews(&projects, false, window, cx);
                    }
                    Ok(Err(error)) => {
                        if !matches!(error, rmcl_app::RemoteContentError::Cancelled) {
                            view.content_browser.search_error = Some(error.to_string());
                        }
                    }
                    Err(error) => {
                        view.content_browser.search_error = Some(
                            rust_i18n::t!("app.common.background_task_failed", error = error)
                                .to_string(),
                        );
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
        if matches!(
            self.content_browser.project_icons.get(&project.id),
            Some(None)
        ) {
            self.content_browser.project_icons.remove(&project.id);
        }
        let gallery_count = if matches!(
            project.kind(),
            Some(ContentKind::ResourcePack | ContentKind::ShaderPack)
        ) {
            project.gallery_previews().len().min(6)
        } else {
            0
        };
        self.content_browser.gallery =
            (gallery_count > 0).then(|| cx.new(|_| CarouselState::new(gallery_count)));
        for url in project.gallery_previews() {
            if matches!(self.content_browser.project_previews.get(url), Some(None)) {
                self.content_browser.project_previews.remove(url);
            }
        }
        self.content_browser.project = Some(project.clone());
        self.content_browser.versions.clear();
        self.content_browser.versions_error = None;
        self.load_project_icons(std::slice::from_ref(&project), window, cx);
        self.load_project_previews(std::slice::from_ref(&project), true, window, cx);
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
        let shader_loader = self.content_browser.shader_loader;
        let loader = project
            .version_loader_filter(version.loader.as_ref().map(|loader| loader.kind.as_str()))
            .or_else(|| {
                (project.kind() == Some(ContentKind::ShaderPack))
                    .then(|| shader_loader.map(shader_loader_filter))
                    .flatten()
                    .map(str::to_owned)
            });
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
                        view.content_browser.versions_error = Some(
                            rust_i18n::t!("app.common.background_task_failed", error = error)
                                .to_string(),
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn back_to_content_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_content_browser_request();
        self.content_browser.project = None;
        self.content_browser.gallery = None;
        self.content_browser.versions.clear();
        self.content_browser.versions_error = None;
        self.content_browser.versions_loading = false;
        let projects = self.content_browser.projects.clone();
        self.load_project_previews(&projects, false, window, cx);
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
                Notification::error(tr("app.content.no_file_for_kind"))
                    .title(tr("app.content.cannot_install")),
                cx,
            );
            return;
        };
        if file.sha1.is_none() {
            window.push_notification(
                Notification::error(tr("app.content.missing_sha1"))
                    .title(tr("app.content.cannot_install")),
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
            .unwrap_or_else(|| content_kind_label(kind));
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
            replace: None,
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
                            Notification::error(error.to_string())
                                .title(tr("app.content.preflight_failed")),
                            cx,
                        );
                    }
                    Err(error) => {
                        window.push_notification(
                            Notification::error(
                                rust_i18n::t!("app.common.background_task_failed", error = error)
                                    .to_string(),
                            )
                            .title(tr("app.content.preflight_failed")),
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
                .join(", ");
            window.push_notification(
                Notification::error(
                    rust_i18n::t!("app.content.incompatible_mods", names = names).to_string(),
                )
                .title(tr("app.content.cannot_install")),
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
                .title(rust_i18n::t!("app.content.install_title", name = display_name).to_string())
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
                                    .child(tr("app.content.required_dependencies")),
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
                                    .child(
                                        rust_i18n::t!(
                                            "app.content.optional_dependencies",
                                            count = dialog_preflight.optional_count
                                        )
                                        .to_string(),
                                    ),
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
                        .child(
                            DialogClose::new()
                                .trigger(|button| button.label(tr("app.common.cancel")).outline()),
                        )
                        .child(
                            DialogAction::new().child(
                                Button::new("confirm-content-preflight")
                                    .primary()
                                    .label(tr("app.common.install")),
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

    fn apply_content_snapshot(&mut self, library: ContentLibrary, inspection: ContentInspection) {
        self.content_icons = inspection
            .resource_packs
            .iter()
            .filter_map(|(path, pack)| {
                let image = image::load_from_memory(pack.metadata.icon_png.as_deref()?).ok()?;
                Some((path.clone(), render_rgba_image(image.to_rgba8())))
            })
            .collect();
        self.content_library = Some(library);
        self.content_inspection = Some(inspection);
    }

    fn load_instance_content(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.content_request = self.content_request.wrapping_add(1);
        let request = self.content_request;
        let Some(version) = self.selected_instance_version().cloned() else {
            self.content_loading = false;
            self.content_library = None;
            self.content_error = Some(tr("app.instance.missing_notice"));
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
                .spawn(async move { scan_and_inspect_instance_content(instance).await })
                .await;
            let _ = this.update_in(cx, |view, _, cx| {
                if view.content_request != request {
                    return;
                }
                view.content_loading = false;
                match result {
                    Ok(Ok((library, inspection))) => {
                        view.apply_content_snapshot(library, inspection);
                        view.content_error = None;
                    }
                    Ok(Err(error)) => view.content_error = Some(error.to_string()),
                    Err(error) => {
                        view.content_error = Some(
                            rust_i18n::t!("app.common.background_task_failed", error = error)
                                .to_string(),
                        );
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
            prompt: Some(
                rust_i18n::t!("app.content.choose_files", kind = content_kind_label(kind))
                    .to_string()
                    .into(),
            ),
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
                            Notification::error(
                                rust_i18n::t!("app.common.file_picker_failed", error = error)
                                    .to_string(),
                            )
                            .title(tr("app.content.file_selection_failed")),
                            cx,
                        );
                    });
                }
                Err(_) => {
                    let _ = this.update_in(cx, |_, window, cx| {
                        window.push_notification(
                            Notification::error(tr("app.content.file_picker_cancelled"))
                                .title(tr("app.content.file_selection_failed")),
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
                    let snapshot = scan_and_inspect_instance_content(instance).await;
                    (imported, failures, snapshot)
                })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.content_action_pending = false;
                match result {
                    Ok((imported, failures, snapshot)) => {
                        match snapshot {
                            Ok((library, inspection)) => {
                                view.apply_content_snapshot(library, inspection);
                            }
                            Err(error) => view.content_error = Some(error.to_string()),
                        }
                        if failures.is_empty() {
                            window.push_notification(
                                Notification::success(
                                    rust_i18n::t!(
                                        "app.content.imported",
                                        count = imported,
                                        kind = content_kind_label(kind)
                                    )
                                    .to_string(),
                                )
                                .title(tr("app.content.import_complete")),
                                cx,
                            );
                        } else {
                            window.push_notification(
                                Notification::warning(
                                    rust_i18n::t!(
                                        "app.content.import_partial",
                                        imported = imported,
                                        failed = failures.len(),
                                        errors = failures.join("; ")
                                    )
                                    .to_string(),
                                )
                                .title(tr("app.content.import_partial_title")),
                                cx,
                            );
                        }
                    }
                    Err(error) => {
                        window.push_notification(
                            Notification::error(
                                rust_i18n::t!("app.common.background_task_failed", error = error)
                                    .to_string(),
                            )
                            .title(tr("app.content.import_failed")),
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
                    let snapshot = scan_and_inspect_instance_content(instance).await;
                    (changed, snapshot)
                })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.content_action_pending = false;
                match result {
                    Ok((changed, snapshot)) => {
                        if let Err(error) = changed {
                            window.push_notification(
                                Notification::error(error.to_string())
                                    .title(tr("app.content.update_failed")),
                                cx,
                            );
                        }
                        match snapshot {
                            Ok((library, inspection)) => {
                                view.apply_content_snapshot(library, inspection);
                            }
                            Err(error) => view.content_error = Some(error.to_string()),
                        }
                    }
                    Err(error) => {
                        window.push_notification(
                            Notification::error(
                                rust_i18n::t!("app.common.background_task_failed", error = error)
                                    .to_string(),
                            )
                            .title(tr("app.content.update_failed")),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn change_resource_pack_priority(
        &mut self,
        relative_path: RepositoryPath,
        order: ResourcePackOrder,
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
                    let changed =
                        move_instance_resource_pack(instance.clone(), relative_path, order).await;
                    let snapshot = scan_and_inspect_instance_content(instance).await;
                    (changed, snapshot)
                })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.content_action_pending = false;
                match result {
                    Ok((changed, snapshot)) => {
                        if let Err(error) = changed {
                            window.push_notification(
                                Notification::error(error.to_string())
                                    .title(tr("app.content.priority_failed")),
                                cx,
                            );
                        }
                        match snapshot {
                            Ok((library, inspection)) => {
                                view.apply_content_snapshot(library, inspection);
                            }
                            Err(error) => view.content_error = Some(error.to_string()),
                        }
                    }
                    Err(error) => {
                        window.push_notification(
                            Notification::error(
                                rust_i18n::t!("app.common.background_task_failed", error = error)
                                    .to_string(),
                            )
                            .title(tr("app.content.priority_failed")),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn select_shader_pack(
        &mut self,
        loader: ShaderLoader,
        pack: Option<String>,
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
                    let changed = set_instance_shader_pack(instance.clone(), loader, pack).await;
                    let snapshot = scan_and_inspect_instance_content(instance).await;
                    (changed, snapshot)
                })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.content_action_pending = false;
                match result {
                    Ok((changed, snapshot)) => {
                        if let Err(error) = changed {
                            window.push_notification(
                                Notification::error(error.to_string())
                                    .title(tr("app.content.shader_failed")),
                                cx,
                            );
                        }
                        match snapshot {
                            Ok((library, inspection)) => {
                                view.apply_content_snapshot(library, inspection);
                            }
                            Err(error) => view.content_error = Some(error.to_string()),
                        }
                    }
                    Err(error) => {
                        window.push_notification(
                            Notification::error(
                                rust_i18n::t!("app.common.background_task_failed", error = error)
                                    .to_string(),
                            )
                            .title(tr("app.content.shader_failed")),
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
                .title(rust_i18n::t!("app.content.delete_title", name = display_name).to_string())
                .description(tr("app.content.delete_description"))
                .ok_text(tr("app.common.delete"))
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
                    let snapshot = scan_and_inspect_instance_content(instance).await;
                    (deleted, snapshot)
                })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.content_action_pending = false;
                match result {
                    Ok((deleted, snapshot)) => match deleted {
                        Ok(()) => {
                            window.push_notification(
                                Notification::success(
                                    rust_i18n::t!("app.content.deleted", name = display_name)
                                        .to_string(),
                                )
                                .title(tr("app.content.deleted_title")),
                                cx,
                            );
                            match snapshot {
                                Ok((library, inspection)) => {
                                    view.apply_content_snapshot(library, inspection);
                                }
                                Err(error) => view.content_error = Some(error.to_string()),
                            }
                        }
                        Err(error) => {
                            window.push_notification(
                                Notification::error(error.to_string())
                                    .title(tr("app.content.delete_failed")),
                                cx,
                            );
                        }
                    },
                    Err(error) => {
                        window.push_notification(
                            Notification::error(
                                rust_i18n::t!("app.common.background_task_failed", error = error)
                                    .to_string(),
                            )
                            .title(tr("app.content.delete_failed")),
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
                Notification::error(
                    rust_i18n::t!("app.content.open_folder_failed", error = error).to_string(),
                )
                .title(tr("app.common.open_failed")),
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
            window.push_notification(
                Notification::error(error).title(tr("app.versions.read_failed")),
                cx,
            );
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
            Page::Worlds => self.render_worlds(cx).into_any_element(),
            Page::Skin => self.render_skin(window, cx).into_any_element(),
            Page::Servers => self.render_servers(cx).into_any_element(),
            Page::Instance => self.render_instance(cx).into_any_element(),
            Page::CreateInstance => self.render_create_instance(cx).into_any_element(),
            Page::ContentBrowser => self.render_content_browser(cx).into_any_element(),
            Page::Agent => self.render_agent(cx).into_any_element(),
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
        ContentPreflightStatus::AlreadyInstalled { enabled: true } => {
            (tr("app.content.status_installed"), Tag::success())
        }
        ContentPreflightStatus::AlreadyInstalled { enabled: false } => {
            (tr("app.content.status_installed_disabled"), Tag::warning())
        }
        ContentPreflightStatus::InstalledDifferentVersion { enabled: true, .. } => {
            (tr("app.content.status_other_version"), Tag::warning())
        }
        ContentPreflightStatus::InstalledDifferentVersion { enabled: false, .. } => (
            tr("app.content.status_other_version_disabled"),
            Tag::warning(),
        ),
        ContentPreflightStatus::WillInstall => (tr("app.content.status_will_install"), Tag::info()),
        ContentPreflightStatus::Conflict => (tr("app.content.status_conflict"), Tag::danger()),
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

fn java_source_label(source: JavaRuntimeSource) -> String {
    match source {
        JavaRuntimeSource::Managed => tr("app.java.source_managed"),
        JavaRuntimeSource::JavaHome => "JAVA_HOME".to_owned(),
        JavaRuntimeSource::System => tr("app.java.source_system"),
        JavaRuntimeSource::Custom => tr("app.java.source_custom"),
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
        Account::Authlib(account) => format!("authlib:{}", account.id()),
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
        AccountKind::Authlib => StoredAccountKind::Authlib,
    }
}

fn authlib_login_error_message(error: &AuthlibAccountServiceError) -> String {
    use AuthlibAccountServiceError as ServiceError;
    match error {
        ServiceError::Auth(AuthlibError::Remote { code, message })
            if code == "ForbiddenOperationException" =>
        {
            if message.trim().is_empty() {
                tr("app.authlib.error_bad_credentials")
            } else {
                rust_i18n::t!("app.authlib.error_login", message = message).to_string()
            }
        }
        ServiceError::Auth(AuthlibError::InvalidUsername) => {
            tr("app.authlib.error_invalid_username")
        }
        ServiceError::Auth(AuthlibError::InvalidPassword) => {
            tr("app.authlib.error_invalid_password")
        }
        ServiceError::Auth(AuthlibError::InvalidServerUrl(message)) => {
            rust_i18n::t!("app.authlib.error_invalid_server", message = message).to_string()
        }
        ServiceError::Auth(AuthlibError::InsecureRedirect) => {
            tr("app.authlib.error_insecure_redirect")
        }
        ServiceError::Auth(AuthlibError::Cancelled) => tr("app.authlib.error_cancelled"),
        ServiceError::CredentialsMissing => tr("app.authlib.error_credentials_missing"),
        ServiceError::ProfileSelectionRequired => tr("app.authlib.error_profile_required"),
        ServiceError::SecretStore(error) => {
            rust_i18n::t!("app.authlib.error_secret_store", error = error).to_string()
        }
        ServiceError::Auth(error) => {
            rust_i18n::t!("app.authlib.error_login_generic", error = error).to_string()
        }
        other => other.to_string(),
    }
}

fn authlib_launch_error_message(error: &AppError) -> String {
    match error {
        AppError::AuthlibAccount(error) => authlib_login_error_message(error),
        AppError::AuthlibInjector(error) => {
            rust_i18n::t!("app.authlib.error_injector", error = error).to_string()
        }
        other => other.to_string(),
    }
}

fn microsoft_error_message(error: &MicrosoftAccountServiceError) -> String {
    match error {
        MicrosoftAccountServiceError::CredentialsMissing
        | MicrosoftAccountServiceError::Auth(MicrosoftAuthError::ReauthenticationRequired)
        | MicrosoftAccountServiceError::Auth(MicrosoftAuthError::MinecraftTokenRejected) => {
            tr("app.account.error_login_expired")
        }
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::NoMinecraftEntitlement) => {
            tr("app.account.error_no_entitlement")
        }
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::ProfileNotFound) => {
            tr("app.account.error_profile_not_found")
        }
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::Xbox {
            code: Some(2148916233),
            ..
        }) => tr("app.account.error_no_xbox_profile"),
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::Xbox {
            code: Some(2148916238),
            ..
        }) => tr("app.account.error_child_account"),
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::Xbox {
            code: Some(2148916235),
            ..
        }) => tr("app.account.error_region"),
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::Xbox {
            code: Some(2148916227),
            ..
        }) => tr("app.account.error_banned"),
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::DeviceCodeExpired) => {
            tr("app.account.error_code_expired")
        }
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::DeviceCodeDeclined) => {
            tr("app.account.error_cancelled")
        }
        MicrosoftAccountServiceError::Auth(MicrosoftAuthError::Cancelled) => {
            tr("app.account.error_cancelled")
        }
        MicrosoftAccountServiceError::SecretStore(error) => {
            rust_i18n::t!("app.account.error_keychain", error = error).to_string()
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

fn theme_label(theme: ThemePreference) -> String {
    tr(match theme {
        ThemePreference::System => "app.settings.theme_system",
        ThemePreference::Light => "app.settings.theme_light",
        ThemePreference::Dark => "app.settings.theme_dark",
    })
}

fn theme_items() -> Vec<String> {
    [
        ThemePreference::System,
        ThemePreference::Light,
        ThemePreference::Dark,
    ]
    .into_iter()
    .map(theme_label)
    .collect()
}

fn download_source_index(source: DownloadSourcePreference) -> usize {
    match source {
        DownloadSourcePreference::Auto => 0,
        DownloadSourcePreference::Official => 1,
        DownloadSourcePreference::Mirror => 2,
    }
}

fn download_source_label(source: DownloadSourcePreference) -> String {
    tr(match source {
        DownloadSourcePreference::Auto => "app.settings.download_source_auto",
        DownloadSourcePreference::Official => "app.settings.download_source_official",
        DownloadSourcePreference::Mirror => "app.settings.download_source_mirror",
    })
}

fn download_source_items() -> Vec<String> {
    [
        DownloadSourcePreference::Auto,
        DownloadSourcePreference::Official,
        DownloadSourcePreference::Mirror,
    ]
    .into_iter()
    .map(download_source_label)
    .collect()
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
        DownloadSourcePreference::Auto => tr("app.mirror.probing_auto"),
        DownloadSourcePreference::Official => tr("app.mirror.official_only"),
        DownloadSourcePreference::Mirror => tr("app.mirror.probing_mirror"),
    }
}

fn mirror_probe_status(source: DownloadSourcePreference, probe: &MirrorProbe) -> String {
    match source {
        DownloadSourcePreference::Official => tr("app.mirror.official_only"),
        DownloadSourcePreference::Auto => match (probe.ok, probe.latency) {
            (true, Some(latency)) => {
                rust_i18n::t!("app.mirror.available", latency = latency.as_millis()).to_string()
            }
            _ => tr("app.mirror.unavailable"),
        },
        DownloadSourcePreference::Mirror => match (probe.ok, probe.latency) {
            (true, Some(latency)) => {
                rust_i18n::t!("app.mirror.preferred", latency = latency.as_millis()).to_string()
            }
            _ => tr("app.mirror.preferred_fallback"),
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

fn visibility_label(visibility: LauncherVisibility) -> String {
    tr(match visibility {
        LauncherVisibility::KeepVisible => "app.settings.visibility_keep",
        LauncherVisibility::HideWhileRunning => "app.settings.visibility_hide",
    })
}

fn visibility_items() -> Vec<String> {
    [
        LauncherVisibility::KeepVisible,
        LauncherVisibility::HideWhileRunning,
    ]
    .into_iter()
    .map(visibility_label)
    .collect()
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

fn accent_label(accent: AccentColor) -> String {
    tr(match accent {
        AccentColor::Emerald => "app.settings.accent_emerald",
        AccentColor::Ocean => "app.settings.accent_ocean",
        AccentColor::Violet => "app.settings.accent_violet",
        AccentColor::Amber => "app.settings.accent_amber",
        AccentColor::Rose => "app.settings.accent_rose",
    })
}

fn accent_items() -> Vec<String> {
    [
        AccentColor::Emerald,
        AccentColor::Ocean,
        AccentColor::Violet,
        AccentColor::Amber,
        AccentColor::Rose,
    ]
    .into_iter()
    .map(accent_label)
    .collect()
}

fn language_index(language: LanguagePreference) -> usize {
    match language {
        LanguagePreference::System => 0,
        LanguagePreference::Chinese => 1,
        LanguagePreference::English => 2,
    }
}

fn language_label(language: LanguagePreference) -> String {
    tr(match language {
        LanguagePreference::System => "app.settings.language_system",
        LanguagePreference::Chinese => "app.settings.language_chinese",
        LanguagePreference::English => "app.settings.language_english",
    })
}

fn language_items() -> Vec<String> {
    [
        LanguagePreference::System,
        LanguagePreference::Chinese,
        LanguagePreference::English,
    ]
    .into_iter()
    .map(language_label)
    .collect()
}

fn authlib_custom_server_label() -> String {
    tr("app.authlib.custom_server")
}

fn authlib_server_items() -> SearchableVec<String> {
    SearchableVec::new(
        AUTHLIB_SERVER_PRESETS
            .iter()
            .map(|preset| preset.name.to_owned())
            .chain(std::iter::once(authlib_custom_server_label()))
            .collect::<Vec<_>>(),
    )
}

pub fn content_kind_label(kind: ContentKind) -> String {
    tr(match kind {
        ContentKind::Mod => "app.content_kind.mod",
        ContentKind::ResourcePack => "app.content_kind.resource_pack",
        ContentKind::ShaderPack => "app.content_kind.shader_pack",
    })
}

pub fn remote_sort_label(sort: RemoteSort) -> String {
    tr(match sort {
        RemoteSort::Relevance => "app.remote_sort.relevance",
        RemoteSort::Downloads => "app.remote_sort.downloads",
        RemoteSort::Follows => "app.remote_sort.follows",
        RemoteSort::Newest => "app.remote_sort.newest",
        RemoteSort::Updated => "app.remote_sort.updated",
    })
}

pub fn skin_sort_label(sort: SkinCatalogSort) -> String {
    tr(match sort {
        SkinCatalogSort::Popular => "app.skin.sort_popular",
        SkinCatalogSort::Newest => "app.skin.sort_newest",
    })
}

pub fn world_compatibility_label(compatibility: WorldCompatibility) -> String {
    tr(match compatibility {
        WorldCompatibility::Same => "app.worlds.compat_same",
        WorldCompatibility::Upgrade => "app.worlds.compat_upgrade",
        WorldCompatibility::Downgrade => "app.worlds.compat_downgrade",
        WorldCompatibility::Unknown => "app.worlds.compat_unknown",
    })
}

pub fn version_type_label(version_type: Option<&str>) -> String {
    tr(match version_type {
        Some("release") => "app.version_filter.release",
        Some("snapshot") => "app.version_filter.snapshot",
        Some("old_beta") => "app.version_filter.old_beta",
        Some("old_alpha") => "app.version_filter.old_alpha",
        Some(_) => "app.version_filter.other",
        None => "app.version_filter.unknown",
    })
}

pub fn short_date(value: Option<&str>) -> String {
    value
        .and_then(|value| value.get(..10))
        .filter(|value| value.len() == 10)
        .map(str::to_owned)
        .unwrap_or_else(|| tr("app.common.date_unknown"))
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
        TaskProgress::Download(progress) => rust_i18n::t!(
            "app.task.progress_download",
            completed = progress.completed_files,
            total = progress.total_files,
            current = display_path(&progress.current)
        )
        .to_string(),
        TaskProgress::Content(progress) => {
            let current = progress
                .current
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| display_path(&progress.current));
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
                Some(total) => rust_i18n::t!(
                    "app.task.progress_content_bytes",
                    position = position,
                    current = current,
                    downloaded = progress.downloaded_bytes,
                    total = total
                )
                .to_string(),
                None => rust_i18n::t!(
                    "app.task.progress_content",
                    position = position,
                    current = current,
                    downloaded = progress.downloaded_bytes
                )
                .to_string(),
            }
        }
        TaskProgress::Processors {
            current,
            completed,
            total,
        } => rust_i18n::t!(
            "app.task.progress_processors",
            completed = completed,
            total = total,
            current = current
        )
        .to_string(),
        TaskProgress::Natives {
            current,
            completed_archives,
            total_archives,
        } => rust_i18n::t!(
            "app.task.progress_natives",
            completed = completed_archives,
            total = total_archives,
            current = display_path(current)
        )
        .to_string(),
    }
}

pub fn task_phase_label(snapshot: Option<&TaskSnapshot>) -> String {
    let Some(phase) = snapshot.and_then(|snapshot| snapshot.phase) else {
        return tr("app.task.ready");
    };
    tr(match phase {
        TaskPhase::Install(InstallPhase::ResolvingVersion) => "app.task.phase_resolving_version",
        TaskPhase::Install(InstallPhase::ReadingVersion) => "app.task.phase_reading_version",
        TaskPhase::Install(InstallPhase::PersistingVersion) => "app.task.phase_persisting_version",
        TaskPhase::Install(InstallPhase::PlanningBase) => "app.task.phase_planning_base",
        TaskPhase::Install(InstallPhase::DownloadingBase) => "app.task.phase_downloading_base",
        TaskPhase::Install(InstallPhase::ReadingAssetIndex) => "app.task.phase_reading_assets",
        TaskPhase::Install(InstallPhase::PlanningAssets) => "app.task.phase_planning_assets",
        TaskPhase::Install(InstallPhase::DownloadingAssets) => "app.task.phase_downloading_assets",
        TaskPhase::Install(InstallPhase::MaterializingAssets) => {
            "app.task.phase_materializing_assets"
        }
        TaskPhase::Install(InstallPhase::DownloadingLoader) => "app.task.phase_downloading_loader",
        TaskPhase::Install(InstallPhase::MergingNatives) => "app.task.phase_merging_natives",
        TaskPhase::Install(InstallPhase::Completed) => "app.task.phase_install_complete",
        TaskPhase::InstanceCreate(InstanceCreatePhase::Preparing) => {
            "app.task.phase_preparing_instance"
        }
        TaskPhase::InstanceCreate(InstanceCreatePhase::InstallingBase) => {
            "app.task.phase_installing_base"
        }
        TaskPhase::InstanceCreate(InstanceCreatePhase::FetchingLoader) => {
            "app.task.phase_fetching_loader"
        }
        TaskPhase::InstanceCreate(InstanceCreatePhase::WritingInstance) => {
            "app.task.phase_writing_instance"
        }
        TaskPhase::InstanceCreate(InstanceCreatePhase::CopyingContent) => {
            "app.task.phase_copying_content"
        }
        TaskPhase::InstanceCreate(InstanceCreatePhase::InstallingLoader) => {
            "app.task.phase_installing_loader"
        }
        TaskPhase::InstanceCreate(InstanceCreatePhase::Completed) => {
            "app.task.instance_create_complete"
        }
        TaskPhase::Content(ContentInstallPhase::Preparing) => "app.task.phase_preparing_content",
        TaskPhase::Content(ContentInstallPhase::Downloading) => {
            "app.task.phase_downloading_content"
        }
        TaskPhase::Content(ContentInstallPhase::Installing) => "app.task.phase_installing_content",
        TaskPhase::Content(ContentInstallPhase::Completed) => "app.task.content_install_complete",
        TaskPhase::Inspecting => "app.task.phase_inspecting",
        TaskPhase::CheckingJava => "app.task.phase_checking_java",
        TaskPhase::InstallingJava => "app.task.phase_installing_java",
        TaskPhase::PreparingLaunch => "app.task.phase_preparing_launch",
        TaskPhase::StartingGame => "app.task.phase_starting_game",
        TaskPhase::RunningGame => "app.task.phase_running_game",
    })
}

pub fn task_status_label(snapshot: Option<&TaskSnapshot>) -> String {
    match snapshot.map(|snapshot| snapshot.status) {
        None => tr("app.task.ready"),
        Some(TaskStatus::Running) => tr("app.task.running"),
        Some(TaskStatus::Cancelling) => tr("app.common.stopping"),
        Some(TaskStatus::Completed)
            if snapshot.map(|snapshot| snapshot.kind) == Some(TaskKind::InstanceCreate) =>
        {
            tr("app.task.instance_create_complete")
        }
        Some(TaskStatus::Completed)
            if snapshot.map(|snapshot| snapshot.kind) == Some(TaskKind::ContentInstall) =>
        {
            tr("app.task.content_install_complete")
        }
        Some(TaskStatus::Completed) => tr("app.task.install_complete"),
        Some(TaskStatus::Cancelled) => tr("app.task.cancelled"),
        Some(TaskStatus::Failed) => tr("app.task.failed"),
        Some(TaskStatus::Exited) => tr("app.task.game_exited"),
    }
}

impl LauncherApp {
    fn refresh_after_external_task(
        &mut self,
        _snapshot: &TaskSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_external_state(window, cx);
        cx.notify();
    }
}

fn shader_loader_filter(loader: ShaderLoader) -> &'static str {
    match loader {
        ShaderLoader::Iris | ShaderLoader::Oculus => "iris",
        ShaderLoader::OptiFine => "optifine",
    }
}

pub fn task_kind_label(kind: TaskKind) -> String {
    tr(match kind {
        TaskKind::VanillaInstall => "app.task.kind_install",
        TaskKind::InstanceCreate => "app.task.kind_instance_create",
        TaskKind::ContentInstall => "app.task.kind_content_install",
        TaskKind::GameLaunch => "app.task.kind_launch",
        TaskKind::JavaInstall => "app.task.kind_java_install",
    })
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

#[cfg(test)]
mod tests {
    use std::{io::Cursor, sync::Arc};

    use gpui::{
        ScrollDelta, ScrollEvent, TestAppContext, VisualTestContext, WindowOptions, point, px,
    };
    use gpui_kit::{AppContext as _, component::WindowExt as _};
    use image::{Rgba, RgbaImage};
    use rmcl_app::{ContentKind, SkinCatalogItem, VersionManifest, prepare_skin};
    use rmcl_auth::SkinVariant;
    use rmcl_store::{ConfigStore, LauncherConfig};
    use tempfile::TempDir;

    use super::{ContentTab, LauncherApp, Page, SkinTab, shortcuts};
    use crate::services::{Bootstrap, Services};

    #[test]
    fn content_browser_tabs_cover_every_content_kind() {
        for kind in ContentKind::ALL {
            let tab = ContentTab::from_kind(kind);
            assert_eq!(tab.kind(), kind);
            assert_eq!(ContentTab::from_index(tab.index()), tab);
        }
    }

    #[gpui::test]
    fn macos_shortcuts_switch_pages_and_control_the_active_window(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            shortcuts::init(cx);
        });
        let temporary = TempDir::new().unwrap();
        let config = LauncherConfig {
            minecraft_root: temporary.path().join("minecraft"),
            java_store: temporary.path().join("java"),
            ..LauncherConfig::default()
        };
        let store = ConfigStore::new(temporary.path().join("config.toml"));
        let services = Services::new(&config, store.clone()).unwrap();
        let bootstrap = Bootstrap {
            config,
            config_store: store,
            config_warning: None,
            save_on_start: false,
        };
        let (handle, app) = cx
            .update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| LauncherApp::new(window, cx, services, bootstrap))
                })
            })
            .unwrap();
        handle
            .update(cx, |_, window, _| window.activate_window())
            .unwrap();
        cx.update(|cx| {
            assert!(cx.active_window().is_some());
            assert!(cx.is_action_available(&shortcuts::Quit));
            assert!(cx.is_action_available(&shortcuts::CloseWindow));
            let menus = cx.get_menus().unwrap();
            let names = menus
                .iter()
                .map(|menu| menu.name.to_string())
                .collect::<Vec<_>>();
            rust_i18n::set_locale("en");
            assert_eq!(names, ["RMCL", "File", "Go", "Window"]);
            assert_eq!(menus[2].items.len(), Page::NAVIGATION.len());
        });
        let mut cx = VisualTestContext::from_window(handle, cx);
        app.update_in(&mut cx, |app, _, cx| {
            app.catalog.manifest = Some(Arc::new(VersionManifest {
                latest: None,
                versions: Vec::new(),
            }));
            app.skin_service.update(cx, |service, _| {
                // Navigating through the skin shortcut must not start real
                // catalog or library I/O inside the deterministic scheduler.
                service.set_catalog_empty_page_for_test(false);
                service.set_recent_for_test(Vec::new());
            });
        });

        cx.dispatch_action(shortcuts::SelectPage {
            page: Page::Versions,
        });
        assert_eq!(cx.read(|cx| app.read(cx).page), Page::Versions);

        for (index, page) in Page::NAVIGATION.into_iter().enumerate() {
            cx.simulate_keystrokes(&format!("cmd-{}", index + 1));
            assert_eq!(
                cx.read(|cx| app.read(cx).page),
                page,
                "shortcut cmd-{}",
                index + 1
            );
        }
        cx.simulate_keystrokes("cmd-,");
        assert_eq!(cx.read(|cx| app.read(cx).page), Page::Settings);

        cx.simulate_keystrokes("ctrl-cmd-f");
        assert!(cx.update(|window, _| window.is_fullscreen()));

        app.update_in(&mut cx, |app, window, cx| {
            app.open_accounts_dialog(window, cx);
        });
        cx.simulate_keystrokes("cmd-2");
        assert_eq!(cx.read(|cx| app.read(cx).page), Page::Versions);
        cx.simulate_keystrokes("cmd-w");
        assert!(cx.read(|cx| cx.windows().is_empty()));
    }

    #[gpui::test]
    fn language_switch_updates_the_active_locale(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            shortcuts::init(cx);
        });
        let temporary = TempDir::new().unwrap();
        let config = LauncherConfig {
            minecraft_root: temporary.path().join("minecraft"),
            java_store: temporary.path().join("java"),
            ..LauncherConfig::default()
        };
        let store = ConfigStore::new(temporary.path().join("config.toml"));
        let services = Services::new(&config, store.clone()).unwrap();
        let bootstrap = Bootstrap {
            config,
            config_store: store,
            config_warning: None,
            save_on_start: false,
        };
        let (handle, app) = cx
            .update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| LauncherApp::new(window, cx, services, bootstrap))
                })
            })
            .unwrap();
        let mut cx = VisualTestContext::from_window(handle, cx);

        app.update_in(&mut cx, |app, window, cx| {
            app.set_language("简体中文".to_owned(), window, cx);
            assert_eq!(&*rust_i18n::locale(), "zh-CN");
            assert_eq!(app.config.language, rmcl_store::LanguagePreference::Chinese);

            app.set_language("English".to_owned(), window, cx);
            assert_eq!(&*rust_i18n::locale(), "en");
            assert_eq!(app.config.language, rmcl_store::LanguagePreference::English);
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        rust_i18n::set_locale("en");
    }

    #[gpui::test]
    fn authlib_login_keeps_the_dialog_open(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let temporary = TempDir::new().unwrap();
        let config = LauncherConfig {
            minecraft_root: temporary.path().join("minecraft"),
            java_store: temporary.path().join("java"),
            ..LauncherConfig::default()
        };
        let store = ConfigStore::new(temporary.path().join("config.toml"));
        let services = Services::new(&config, store.clone()).unwrap();
        let bootstrap = Bootstrap {
            config,
            config_store: store,
            config_warning: None,
            save_on_start: false,
        };
        let (handle, app) = cx
            .update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| LauncherApp::new(window, cx, services, bootstrap))
                })
            })
            .unwrap();
        let mut cx = VisualTestContext::from_window(handle, cx);

        app.update_in(&mut cx, |app, window, cx| {
            app.open_authlib_login_dialog(window, cx);
            app.authlib_custom_server = true;
            app.authlib_server_input.update(cx, |state, cx| {
                state.set_value("http://127.0.0.1:1", window, cx);
            });
            app.authlib_username_input.update(cx, |state, cx| {
                state.set_value("user", window, cx);
            });
            app.authlib_password_input.update(cx, |state, cx| {
                state.set_value("pass", window, cx);
            });
            assert!(!app.begin_authlib_login(window, cx));
        });

        assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
    }

    #[gpui::test]
    fn agent_wheel_reaches_the_transcript_tail(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let temporary = TempDir::new().unwrap();
        let config = LauncherConfig {
            minecraft_root: temporary.path().join("minecraft"),
            java_store: temporary.path().join("java"),
            ..LauncherConfig::default()
        };
        let store = ConfigStore::new(temporary.path().join("config.toml"));
        let services = Services::new(&config, store.clone()).unwrap();
        let bootstrap = Bootstrap {
            config,
            config_store: store,
            config_warning: None,
            save_on_start: false,
        };
        let (handle, app) = cx
            .update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| LauncherApp::new(window, cx, services, bootstrap))
                })
            })
            .unwrap();
        let mut cx = VisualTestContext::from_window(handle, cx);

        app.update_in(&mut cx, |app, window, cx| {
            app.navigate(Page::Agent, window, cx);
            for index in 0..40 {
                app.agent.push_user(
                    format!("Message {index}: a long transcript entry that wraps across lines."),
                    cx,
                );
            }
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let viewport = cx
            .debug_bounds("agent-scroll-viewport")
            .expect("transcript viewport was not laid out");
        cx.simulate_event(ScrollEvent {
            position: viewport.center(),
            delta: ScrollDelta::Pixels(point(px(0.), px(-100_000.))),
            ..Default::default()
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let last = cx
            .debug_bounds("agent-last-message")
            .expect("last message row was not laid out");
        assert!(
            last.top() >= viewport.top() - px(0.5),
            "last message starts above the viewport: {last:?} vs {viewport:?}"
        );
        assert!(
            last.bottom() <= viewport.bottom() + px(0.5),
            "wheel stopped before the last message: {last:?} vs {viewport:?}"
        );
        let tail = cx
            .debug_bounds("agent-scroll-tail")
            .expect("scroll tail row was not laid out");
        assert!(
            tail.bottom() <= viewport.bottom() + px(8.5),
            "wheel stopped before the measured tail: {tail:?} vs {viewport:?}"
        );
        let scrolled_up = cx.read(|cx| {
            app.read(cx)
                .agent
                .conversation
                .scroller
                .read(cx)
                .is_scrolled_up()
        });
        assert!(!scrolled_up, "wheel stopped away from the tail");
    }

    #[gpui::test]
    fn skin_page_renders_online_library_surface(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let temporary = TempDir::new().unwrap();
        let config = LauncherConfig {
            minecraft_root: temporary.path().join("minecraft"),
            java_store: temporary.path().join("java"),
            ..LauncherConfig::default()
        };
        let store = ConfigStore::new(temporary.path().join("config.toml"));
        let services = Services::new(&config, store.clone()).unwrap();
        let bootstrap = Bootstrap {
            config,
            config_store: store,
            config_warning: None,
            save_on_start: false,
        };
        let (handle, app) = cx
            .update(|cx| {
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds {
                            origin: Default::default(),
                            size: gpui_kit::size(px(1180.), px(780.)),
                        })),
                        focus: false,
                        show: false,
                        ..Default::default()
                    },
                    cx,
                    |window, cx| cx.new(|cx| LauncherApp::new(window, cx, services, bootstrap)),
                )
            })
            .unwrap();
        let mut cx = VisualTestContext::from_window(handle, cx);

        let mut png = Vec::new();
        RgbaImage::from_pixel(64, 64, Rgba([90, 120, 180, 255]))
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let prepared = prepare_skin(&png, "test.png", None).unwrap();
        let thumbnail = super::ui::common::render_rgba_image(prepared.texture().image().clone());
        let items = vec![
            SkinCatalogItem::new(
                "littleskin:1",
                1,
                "测试皮肤",
                Some("tester".to_owned()),
                SkinVariant::Classic,
                42,
                "https://littleskin.cn/skinlib/show/1",
            ),
            SkinCatalogItem::new(
                "littleskin:2",
                2,
                "第二张",
                None,
                SkinVariant::Slim,
                7,
                "https://littleskin.cn/skinlib/show/2",
            ),
        ];
        app.update_in(&mut cx, |app, _, cx| {
            app.page = Page::Skin;
            app.skin_tab = SkinTab::Online;
            app.skin_service.update(cx, |service, _| {
                service.set_catalog_for_test(items, thumbnail, prepared);
            });
            cx.notify();
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        let card = cx
            .debug_bounds("skin-library-card")
            .expect("skin library card was not laid out");
        assert!(card.size.width > px(300.));
        assert!(card.size.height > px(400.));
        let first = cx
            .debug_bounds("skin-catalog-card-littleskin:1")
            .expect("first online skin card was not laid out");
        let second = cx
            .debug_bounds("skin-catalog-card-littleskin:2")
            .expect("second online skin card was not laid out");
        assert!(first.size.width > px(120.));
        assert!(first.size.height > px(180.));
        assert_eq!(first.size.height, second.size.height);

        app.update_in(&mut cx, |app, _, cx| {
            app.skin_service.update(cx, |service, _| {
                service.set_catalog_empty_page_for_test(true);
            });
            cx.notify();
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(
            cx.debug_bounds("skin-catalog-more-empty").is_some(),
            "a filtered empty page should still offer the next page"
        );

        app.update_in(&mut cx, |app, _, cx| {
            app.skin_service.update(cx, |service, _| {
                service.set_catalog_empty_error_for_test(0, true, "网络错误");
            });
            cx.notify();
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(
            cx.debug_bounds("skin-catalog-error-retry").is_some(),
            "an empty error state should offer a retry"
        );
        assert!(
            cx.debug_bounds("skin-catalog-more-empty").is_none(),
            "an error must not be presented as a filtered page"
        );

        app.update_in(&mut cx, |app, _, cx| {
            app.skin_service.update(cx, |service, _| {
                service.set_catalog_failed_for_test("皮肤不存在");
                assert!(!service.catalog().is_selected("littleskin:42"));
                assert_eq!(service.catalog().selection_error(), Some("皮肤不存在"));
            });
        });
    }
}
