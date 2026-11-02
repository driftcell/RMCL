//! Shared official-skin profile and texture cache.
//!
//! Profiles are cached per account so leaving and re-entering the skin page
//! does not re-fetch them. Each mutually exclusive async operation lives in a
//! small state enum with its own task handle.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime},
};

use gpui_kit::*;
use image::{Rgba, RgbaImage};
use rmcl_app::{
    CatalogSkin, SkinCatalogError, SkinCatalogItem, SkinCatalogSort, SkinError, SkinLibraryEntry,
    SkinPreviewRenderer, SkinTexture,
};
use rmcl_auth::{MicrosoftAccount, SkinVariant};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{i18n::tr, services::Services};

const PROFILE_MAX_AGE: Duration = Duration::from_secs(60);

/// A profile snapshot with the time it was fetched.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct LoadedProfile {
    pub profile: MicrosoftAccount,
    pub synced_at: SystemTime,
}

impl LoadedProfile {
    fn is_fresh(&self) -> bool {
        self.synced_at.elapsed().unwrap_or_default() < PROFILE_MAX_AGE
    }
}

/// The texture currently equipped by an account.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct EquippedSkin {
    pub url: Option<String>,
    pub texture: SkinTexture,
    pub variant: SkinVariant,
}

/// A generic async resource whose loading task is owned by the state.
pub enum Loadable<T> {
    NotLoaded,
    Loading {
        previous: Option<T>,
        _task: Task<()>,
    },
    Ready(T),
    Failed {
        error: String,
        previous: Option<T>,
    },
}

impl<T> Loadable<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Ready(value) => Some(value),
            Self::Loading { previous, .. } | Self::Failed { previous, .. } => previous.as_ref(),
            Self::NotLoaded => None,
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading { .. })
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Failed { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// Profile loading state for one account.
pub enum ProfileState {
    Loading {
        account: Uuid,
        request: u64,
        previous: Option<LoadedProfile>,
        _task: Task<()>,
    },
    Loaded(LoadedProfile),
    Failed {
        error: String,
        previous: Option<LoadedProfile>,
    },
}

impl ProfileState {
    pub fn value(&self) -> Option<&LoadedProfile> {
        match self {
            Self::Loaded(profile) => Some(profile),
            Self::Loading { previous, .. } | Self::Failed { previous, .. } => previous.as_ref(),
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading { .. })
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Failed { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// Texture loading state for one account.
pub enum EquippedSkinState {
    Loading {
        url: String,
        previous: Option<EquippedSkin>,
        _task: Task<()>,
    },
    Ready(EquippedSkin),
    Failed {
        error: String,
        previous: Option<EquippedSkin>,
    },
}

impl EquippedSkinState {
    pub fn value(&self) -> Option<&EquippedSkin> {
        match self {
            Self::Ready(skin) => Some(skin),
            Self::Loading { previous, .. } | Self::Failed { previous, .. } => previous.as_ref(),
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Failed { error, .. } => Some(error),
            _ => None,
        }
    }

    fn matches_url(&self, url: &str) -> bool {
        match self {
            Self::Loading { url: current, .. } => current == url,
            Self::Ready(skin) => skin.url.as_deref() == Some(url),
            Self::Failed { .. } => false,
        }
    }
}

/// A downloaded catalog skin ready to become the current preview candidate.
#[derive(Debug, Clone)]
pub enum SkinServiceEvent {
    ProfileLoaded(LoadedProfile),
    CatalogSkinLoaded(CatalogSkin),
}

/// The rendered preview for one online catalog card.
pub enum CatalogThumbState {
    Loading,
    Ready {
        image: Arc<RenderImage>,
        skin: rmcl_app::PreparedSkin,
    },
    Failed,
}

/// The texture download for the catalog card the user selected.
enum CatalogSelectionState {
    Idle,
    Loading {
        item_id: String,
        _task: Task<()>,
    },
    Ready {
        item_id: String,
        skin: rmcl_app::PreparedSkin,
    },
}

/// Online catalog results and their local 3D thumbnails.
pub struct SkinCatalogState {
    items: Vec<SkinCatalogItem>,
    thumbnails: HashMap<String, CatalogThumbState>,
    page: u32,
    has_more: bool,
    loading: bool,
    error: Option<String>,
    query: String,
    sort: SkinCatalogSort,
    selection: CatalogSelectionState,
    selection_error: Option<String>,
    thumbnail_limiter: Arc<Semaphore>,
    load_cancel: CancellationToken,
    thumbnail_cancel: CancellationToken,
    selection_cancel: CancellationToken,
    _load_task: Option<Task<()>>,
    thumbnail_tasks: Vec<Task<()>>,
}

impl Default for SkinCatalogState {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            thumbnails: HashMap::new(),
            page: 0,
            has_more: true,
            loading: false,
            error: None,
            query: String::new(),
            sort: SkinCatalogSort::default(),
            selection: CatalogSelectionState::Idle,
            selection_error: None,
            thumbnail_limiter: Arc::new(Semaphore::new(4)),
            load_cancel: CancellationToken::new(),
            thumbnail_cancel: CancellationToken::new(),
            selection_cancel: CancellationToken::new(),
            _load_task: None,
            thumbnail_tasks: Vec::new(),
        }
    }
}

impl SkinCatalogState {
    pub fn items(&self) -> &[SkinCatalogItem] {
        &self.items
    }

    pub fn thumbnail(&self, item_id: &str) -> Option<&CatalogThumbState> {
        self.thumbnails.get(item_id)
    }

    pub fn page(&self) -> u32 {
        self.page
    }

    pub fn has_more(&self) -> bool {
        self.has_more
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The item whose preview is loading or ready.
    pub fn selected_item(&self) -> Option<&str> {
        match &self.selection {
            CatalogSelectionState::Loading { item_id, .. }
            | CatalogSelectionState::Ready { item_id, .. } => Some(item_id),
            CatalogSelectionState::Idle => None,
        }
    }

    pub fn is_selected(&self, item_id: &str) -> bool {
        self.selected_item() == Some(item_id)
    }

    pub fn is_selecting(&self, item_id: &str) -> bool {
        matches!(
            &self.selection,
            CatalogSelectionState::Loading { item_id: current, .. } if current == item_id
        )
    }

    pub fn selection_error(&self) -> Option<&str> {
        self.selection_error.as_deref()
    }
}

impl EventEmitter<SkinServiceEvent> for SkinService {}

/// Per-account skin profiles, textures and the local recent list.
pub struct SkinService {
    services: Arc<Services>,
    profiles: HashMap<Uuid, ProfileState>,
    textures: HashMap<Uuid, EquippedSkinState>,
    recent: Loadable<Vec<SkinLibraryEntry>>,
    catalog: SkinCatalogState,
    next_request: u64,
    recent_request: u64,
    catalog_request: u64,
}

impl SkinService {
    pub fn new(services: Arc<Services>, _cx: &mut Context<Self>) -> Self {
        Self {
            services,
            profiles: HashMap::new(),
            textures: HashMap::new(),
            recent: Loadable::NotLoaded,
            catalog: SkinCatalogState::default(),
            next_request: 0,
            recent_request: 0,
            catalog_request: 0,
        }
    }

    pub fn catalog(&self) -> &SkinCatalogState {
        &self.catalog
    }

    /// Loads the first catalog page once. A failed empty page can be retried by
    /// entering the page again; explicit searches always call [`Self::reload_catalog`].
    pub fn ensure_catalog(&mut self, cx: &mut Context<Self>) {
        if !self.catalog.loading && self.catalog.page == 0 {
            let query = self.catalog.query.clone();
            let sort = self.catalog.sort;
            self.reload_catalog(query, sort, cx);
        }
    }

    /// Replaces the results with a new query and sort order.
    pub fn reload_catalog(&mut self, query: String, sort: SkinCatalogSort, cx: &mut Context<Self>) {
        self.catalog_request = self.catalog_request.wrapping_add(1);
        self.catalog.query = query;
        self.catalog.sort = sort;
        self.catalog.items.clear();
        self.catalog.thumbnails.clear();
        self.catalog.thumbnail_tasks.clear();
        restart_cancellation(&mut self.catalog.thumbnail_cancel);
        restart_cancellation(&mut self.catalog.selection_cancel);
        self.catalog.page = 0;
        self.catalog.has_more = true;
        self.catalog.error = None;
        self.catalog.selection = CatalogSelectionState::Idle;
        self.catalog.selection_error = None;
        self.start_catalog_page(1, false, cx);
    }

    /// Appends the next catalog page.
    pub fn load_more_catalog(&mut self, cx: &mut Context<Self>) {
        if self.catalog.loading || !self.catalog.has_more {
            return;
        }
        self.start_catalog_page(self.catalog.page.saturating_add(1).max(1), true, cx);
    }

    /// Starts or reuses the texture download for one catalog card.
    pub fn select_catalog_item(&mut self, item: SkinCatalogItem, cx: &mut Context<Self>) {
        if let Some(CatalogThumbState::Ready { skin, .. }) = self.catalog.thumbnail(&item.id) {
            let skin = skin.clone();
            restart_cancellation(&mut self.catalog.selection_cancel);
            self.catalog.selection = CatalogSelectionState::Ready {
                item_id: item.id.clone(),
                skin: skin.clone(),
            };
            self.catalog.selection_error = None;
            cx.emit(SkinServiceEvent::CatalogSkinLoaded(CatalogSkin::new(
                item, skin,
            )));
            cx.notify();
            return;
        }
        if let CatalogSelectionState::Ready {
            item_id,
            skin: previous,
        } = &self.catalog.selection
            && item_id == &item.id
        {
            let skin = previous.clone();
            restart_cancellation(&mut self.catalog.selection_cancel);
            self.catalog.selection_error = None;
            cx.emit(SkinServiceEvent::CatalogSkinLoaded(CatalogSkin::new(
                item,
                skin.clone(),
            )));
            cx.notify();
            return;
        }
        if self.catalog.is_selecting(&item.id) {
            return;
        }

        let item_id = item.id.clone();
        let item_id_for_task = item_id.clone();
        let previous = match &self.catalog.selection {
            CatalogSelectionState::Ready { item_id, skin } => Some((item_id.clone(), skin.clone())),
            _ => None,
        };
        let catalog = self.services.skin_catalog.clone();
        let urls = self.services.skin_urls.clone();
        let runtime = self.services.runtime.handle().clone();
        let cancellation = restart_cancellation(&mut self.catalog.selection_cancel);
        self.catalog.selection_error = None;
        let task = cx.spawn(async move |this, cx| {
            let item_for_task = item.clone();
            let result = runtime
                .spawn(async move { catalog.prepare(&item_for_task, &urls, &cancellation).await })
                .await;
            let _ = this.update(cx, |this, cx| {
                if !this.catalog.is_selecting(&item_id_for_task) {
                    return;
                }
                match result {
                    Ok(Ok(skin)) => {
                        this.catalog.selection = CatalogSelectionState::Ready {
                            item_id: item_id_for_task.clone(),
                            skin: skin.clone(),
                        };
                        this.catalog.selection_error = None;
                        cx.emit(SkinServiceEvent::CatalogSkinLoaded(CatalogSkin::new(
                            item, skin,
                        )));
                    }
                    Ok(Err(error)) => {
                        this.catalog.selection_error = Some(describe_catalog_error(&error));
                        this.catalog.selection = restore_previous_selection(previous.clone());
                    }
                    Err(error) => {
                        this.catalog.selection_error =
                            Some(rust_i18n::t!("app.skin.task_failed", error = error).to_string());
                        this.catalog.selection = restore_previous_selection(previous.clone());
                    }
                }
                cx.notify();
            });
        });
        self.catalog.selection = CatalogSelectionState::Loading {
            item_id,
            _task: task,
        };
        cx.notify();
    }

    pub fn clear_catalog_selection(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.catalog.selection, CatalogSelectionState::Idle)
            || self.catalog.selection_error.is_some()
        {
            restart_cancellation(&mut self.catalog.selection_cancel);
            self.catalog.selection = CatalogSelectionState::Idle;
            self.catalog.selection_error = None;
            cx.notify();
        }
    }

    #[cfg(test)]
    pub(crate) fn set_catalog_for_test(
        &mut self,
        items: Vec<SkinCatalogItem>,
        image: Arc<RenderImage>,
        skin: rmcl_app::PreparedSkin,
    ) {
        self.catalog.items = items.clone();
        self.catalog.page = 1;
        self.catalog.has_more = false;
        self.catalog.loading = false;
        self.catalog.error = None;
        self.catalog.selection_error = None;
        self.catalog.thumbnails = items
            .into_iter()
            .map(|item| {
                (
                    item.id,
                    CatalogThumbState::Ready {
                        image: image.clone(),
                        skin: skin.clone(),
                    },
                )
            })
            .collect();
    }

    #[cfg(test)]
    pub(crate) fn set_catalog_empty_page_for_test(&mut self, has_more: bool) {
        self.catalog.items.clear();
        self.catalog.thumbnails.clear();
        self.catalog.page = 1;
        self.catalog.has_more = has_more;
        self.catalog.loading = false;
        self.catalog.error = None;
        self.catalog.selection = CatalogSelectionState::Idle;
        self.catalog.selection_error = None;
    }

    #[cfg(test)]
    pub(crate) fn set_catalog_empty_error_for_test(
        &mut self,
        page: u32,
        has_more: bool,
        error: &str,
    ) {
        self.catalog.items.clear();
        self.catalog.thumbnails.clear();
        self.catalog.page = page;
        self.catalog.has_more = has_more;
        self.catalog.loading = false;
        self.catalog.error = Some(error.to_owned());
        self.catalog.selection = CatalogSelectionState::Idle;
        self.catalog.selection_error = None;
    }

    #[cfg(test)]
    pub(crate) fn set_catalog_failed_for_test(&mut self, error: &str) {
        self.catalog.selection = CatalogSelectionState::Idle;
        self.catalog.selection_error = Some(error.to_owned());
    }

    #[cfg(test)]
    pub(crate) fn set_recent_for_test(&mut self, entries: Vec<SkinLibraryEntry>) {
        self.recent = Loadable::Ready(entries);
    }

    /// Removes rendered thumbnails before a search so the caller can release
    /// their GPUI atlas entries with `Window::drop_image`.
    pub fn take_catalog_thumbnails(&mut self) -> Vec<Arc<RenderImage>> {
        let mut images = Vec::new();
        for state in self.catalog.thumbnails.drain() {
            if let (_, CatalogThumbState::Ready { image, .. }) = state {
                images.push(image);
            }
        }
        images
    }

    pub fn profile(&self, account: Uuid) -> Option<&LoadedProfile> {
        self.profiles.get(&account).and_then(ProfileState::value)
    }

    pub fn profile_state(&self, account: Uuid) -> Option<&ProfileState> {
        self.profiles.get(&account)
    }

    pub fn equipped_skin(&self, account: Uuid) -> Option<&EquippedSkin> {
        self.textures
            .get(&account)
            .and_then(EquippedSkinState::value)
    }

    pub fn texture_state(&self, account: Uuid) -> Option<&EquippedSkinState> {
        self.textures.get(&account)
    }

    pub fn recent(&self) -> &Loadable<Vec<SkinLibraryEntry>> {
        &self.recent
    }

    /// Drops cached state for a deleted account and cancels its tasks.
    pub fn remove_account(&mut self, account: Uuid, cx: &mut Context<Self>) {
        self.profiles.remove(&account);
        self.textures.remove(&account);
        cx.notify();
    }

    /// Loads a profile unless a fresh or in-flight copy already exists.
    pub fn ensure_profile(&mut self, account: MicrosoftAccount, cx: &mut Context<Self>) {
        let uuid = account.uuid();
        if let Some(state) = self.profiles.get(&uuid)
            && (state.is_loading()
                || matches!(state, ProfileState::Loaded(profile) if profile.is_fresh()))
        {
            return;
        }
        self.start_profile_load(account, cx);
    }

    /// Forces a profile refresh while retaining the previous snapshot.
    pub fn refresh_profile(&mut self, account: MicrosoftAccount, cx: &mut Context<Self>) {
        self.start_profile_load(account, cx);
    }

    /// Accepts a profile returned by a skin mutation and updates the texture.
    pub fn accept_profile(
        &mut self,
        profile: MicrosoftAccount,
        texture: Option<(SkinTexture, SkinVariant)>,
        cx: &mut Context<Self>,
    ) {
        let uuid = profile.uuid();
        let loaded = LoadedProfile {
            profile: profile.clone(),
            synced_at: SystemTime::now(),
        };
        self.profiles
            .insert(uuid, ProfileState::Loaded(loaded.clone()));
        if let Some((texture, variant)) = texture {
            self.textures.insert(
                uuid,
                EquippedSkinState::Ready(EquippedSkin {
                    url: profile.skin_url().map(str::to_owned),
                    texture,
                    variant,
                }),
            );
        } else {
            self.ensure_equipped_skin(uuid, &profile, cx);
        }
        cx.emit(SkinServiceEvent::ProfileLoaded(loaded));
        cx.notify();
    }

    pub fn ensure_recent(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.recent, Loadable::NotLoaded | Loadable::Failed { .. }) {
            return;
        }
        self.start_recent_load(cx);
    }

    pub fn reload_recent(&mut self, cx: &mut Context<Self>) {
        self.start_recent_load(cx);
    }

    fn start_profile_load(&mut self, account: MicrosoftAccount, cx: &mut Context<Self>) {
        let uuid = account.uuid();
        let previous = self.profile(uuid).cloned();
        let previous_for_state = previous.clone();
        let request = self.next_request;
        self.next_request = self.next_request.wrapping_add(1);
        let service = self.services.microsoft_accounts.clone();
        let runtime = self.services.runtime.handle().clone();
        let task = cx.spawn(async move |this, cx| {
            let cancellation = CancellationToken::new();
            let account_for_task = account.clone();
            let result = runtime
                .spawn(async move {
                    service
                        .refresh_profile(&account_for_task, &cancellation)
                        .await
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let still_loading = matches!(
                    this.profiles.get(&uuid),
                    Some(ProfileState::Loading {
                        account,
                        request: current_request,
                        ..
                    }) if *account == uuid && *current_request == request
                );
                if !still_loading {
                    return;
                }
                match result {
                    Ok(Ok(profile)) => {
                        let loaded = LoadedProfile {
                            profile: profile.clone(),
                            synced_at: SystemTime::now(),
                        };
                        this.profiles
                            .insert(uuid, ProfileState::Loaded(loaded.clone()));
                        this.ensure_equipped_skin(uuid, &profile, cx);
                        cx.emit(SkinServiceEvent::ProfileLoaded(loaded));
                    }
                    Ok(Err(error)) => {
                        this.profiles.insert(
                            uuid,
                            ProfileState::Failed {
                                error: describe_profile_error(&error),
                                previous,
                            },
                        );
                    }
                    Err(error) => {
                        this.profiles.insert(
                            uuid,
                            ProfileState::Failed {
                                error: rust_i18n::t!("app.skin.profile_task_failed", error = error)
                                    .to_string(),
                                previous,
                            },
                        );
                    }
                }
                cx.notify();
            });
        });
        self.profiles.insert(
            uuid,
            ProfileState::Loading {
                account: uuid,
                request,
                previous: previous_for_state,
                _task: task,
            },
        );
        cx.notify();
    }

    fn ensure_equipped_skin(
        &mut self,
        account: Uuid,
        profile: &MicrosoftAccount,
        cx: &mut Context<Self>,
    ) {
        let Some(url) = profile.skin_url().map(str::to_owned) else {
            let Some(texture) = placeholder_skin() else {
                self.textures.insert(
                    account,
                    EquippedSkinState::Failed {
                        error: tr("app.skin.default_preview_failed"),
                        previous: None,
                    },
                );
                return;
            };
            self.textures.insert(
                account,
                EquippedSkinState::Ready(EquippedSkin {
                    url: None,
                    texture,
                    variant: SkinVariant::Classic,
                }),
            );
            cx.notify();
            return;
        };

        if self
            .textures
            .get(&account)
            .is_some_and(|state| state.matches_url(&url))
        {
            return;
        }
        let variant = profile
            .active_skin()
            .map(|skin| skin.variant())
            .unwrap_or(SkinVariant::Classic);
        let previous = self.equipped_skin(account).cloned();
        let previous_for_state = previous.clone();
        let skins = self.services.skins.clone();
        let runtime = self.services.runtime.handle().clone();
        let url_for_task = url.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn({
                    let skins = skins.clone();
                    let url = url_for_task.clone();
                    async move { skins.ensure_texture(&url).await }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let still_loading = matches!(
                    this.textures.get(&account),
                    Some(EquippedSkinState::Loading { url, .. }) if url == &url_for_task
                );
                if !still_loading {
                    return;
                }
                match result {
                    Ok(Ok(texture)) => {
                        this.textures.insert(
                            account,
                            EquippedSkinState::Ready(EquippedSkin {
                                url: Some(url_for_task),
                                texture,
                                variant,
                            }),
                        );
                    }
                    Ok(Err(error)) => {
                        this.textures.insert(
                            account,
                            EquippedSkinState::Failed {
                                error: describe_texture_error(&error),
                                previous,
                            },
                        );
                    }
                    Err(error) => {
                        this.textures.insert(
                            account,
                            EquippedSkinState::Failed {
                                error: rust_i18n::t!("app.skin.texture_task_failed", error = error)
                                    .to_string(),
                                previous,
                            },
                        );
                    }
                }
                cx.notify();
            });
        });
        self.textures.insert(
            account,
            EquippedSkinState::Loading {
                url,
                previous: previous_for_state,
                _task: task,
            },
        );
        cx.notify();
    }

    fn start_catalog_page(&mut self, page: u32, append: bool, cx: &mut Context<Self>) {
        let request = self.catalog_request;
        let query = self.catalog.query.clone();
        let sort = self.catalog.sort;
        let catalog = self.services.skin_catalog.clone();
        let runtime = self.services.runtime.handle().clone();
        let cancellation = restart_cancellation(&mut self.catalog.load_cancel);
        self.catalog.loading = true;
        self.catalog.error = None;
        cx.notify();

        let task = cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async move { catalog.list(&query, sort, page, &cancellation).await })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.catalog_request != request {
                    return;
                }
                this.catalog.loading = false;
                match result {
                    Ok(Ok(page)) => {
                        let new_items = if append {
                            this.catalog.items.extend(page.items.iter().cloned());
                            page.items
                        } else {
                            this.catalog.items = page.items.clone();
                            page.items
                        };
                        this.catalog.page = page.page;
                        this.catalog.has_more = page.has_more;
                        this.start_catalog_thumbnails(new_items, cx);
                    }
                    Ok(Err(error)) => {
                        this.catalog.error = Some(describe_catalog_error(&error));
                    }
                    Err(error) => {
                        this.catalog.error =
                            Some(rust_i18n::t!("app.skin.task_failed", error = error).to_string());
                    }
                }
                cx.notify();
            });
        });
        self.catalog._load_task = Some(task);
    }

    fn start_catalog_thumbnails(&mut self, items: Vec<SkinCatalogItem>, cx: &mut Context<Self>) {
        let semaphore = self.catalog.thumbnail_limiter.clone();
        let cancellation = self.catalog.thumbnail_cancel.clone();
        let catalog = self.services.skin_catalog.clone();
        let urls = self.services.skin_urls.clone();
        let runtime = self.services.runtime.handle().clone();
        for item in items {
            if self.catalog.thumbnails.contains_key(&item.id) {
                continue;
            }
            self.catalog
                .thumbnails
                .insert(item.id.clone(), CatalogThumbState::Loading);
            let item_id = item.id.clone();
            let item_for_task = item.clone();
            let semaphore = semaphore.clone();
            let cancellation = cancellation.clone();
            let catalog = catalog.clone();
            let urls = urls.clone();
            let runtime = runtime.clone();
            let task = cx.spawn(async move |this, cx| {
                let _permit = semaphore.acquire_owned().await.ok();
                if cancellation.is_cancelled() {
                    return;
                }
                let result = runtime
                    .spawn(async move {
                        let skin = catalog
                            .prepare(&item_for_task, &urls, &cancellation)
                            .await?;
                        let texture = skin.texture().clone();
                        let variant = item_for_task.variant;
                        let image = tokio::task::spawn_blocking(move || {
                            render_catalog_thumbnail(&texture, variant)
                        })
                        .await
                        .map_err(|error| SkinCatalogError::Blocking(error.to_string()))?;
                        Ok::<_, SkinCatalogError>((skin, image))
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if !matches!(
                        this.catalog.thumbnails.get(&item_id),
                        Some(CatalogThumbState::Loading)
                    ) {
                        return;
                    }
                    this.catalog.thumbnails.insert(
                        item_id,
                        match result {
                            Ok(Ok((skin, image))) => CatalogThumbState::Ready { image, skin },
                            _ => CatalogThumbState::Failed,
                        },
                    );
                    cx.notify();
                });
            });
            self.catalog.thumbnail_tasks.push(task);
        }
    }

    fn start_recent_load(&mut self, cx: &mut Context<Self>) {
        let previous = self.recent.value().cloned();
        let previous_for_state = previous.clone();
        self.recent_request = self.recent_request.wrapping_add(1);
        let request = self.recent_request;
        let library = self.services.skin_library.clone();
        let runtime = self.services.runtime.handle().clone();
        let task = cx.spawn(async move |this, cx| {
            let result = runtime.spawn(async move { library.list().await }).await;
            let _ = this.update(cx, |this, cx| {
                if this.recent_request != request || !this.recent.is_loading() {
                    return;
                }
                this.recent = match result {
                    Ok(Ok(entries)) => Loadable::Ready(entries),
                    Ok(Err(error)) => Loadable::Failed {
                        error: rust_i18n::t!("app.skin.recent_read_failed", error = error)
                            .to_string(),
                        previous,
                    },
                    Err(error) => Loadable::Failed {
                        error: rust_i18n::t!("app.skin.recent_task_failed", error = error)
                            .to_string(),
                        previous,
                    },
                };
                cx.notify();
            });
        });
        self.recent = Loadable::Loading {
            previous: previous_for_state,
            _task: task,
        };
        cx.notify();
    }
}

/// Cancels in-flight inner runtime work before the next request replaces it.
///
/// Dropping the GPUI task cancels the outer future, but the `tokio` task spawned
/// inside it would otherwise keep running and downloading detached.
fn restart_cancellation(token: &mut CancellationToken) -> CancellationToken {
    token.cancel();
    *token = CancellationToken::new();
    token.clone()
}

fn restore_previous_selection(
    previous: Option<(String, rmcl_app::PreparedSkin)>,
) -> CatalogSelectionState {
    match previous {
        Some((item_id, skin)) => CatalogSelectionState::Ready { item_id, skin },
        None => CatalogSelectionState::Idle,
    }
}

fn describe_profile_error(error: &rmcl_app::MicrosoftAccountServiceError) -> String {
    use rmcl_app::MicrosoftAccountServiceError;
    use rmcl_auth::MicrosoftAuthError;

    match error {
        MicrosoftAccountServiceError::CredentialsMissing
        | MicrosoftAccountServiceError::Auth(
            MicrosoftAuthError::ReauthenticationRequired
            | MicrosoftAuthError::MinecraftTokenRejected,
        ) => tr("app.skin.error_login_expired_short"),
        MicrosoftAccountServiceError::SecretStore(_) => tr("app.skin.error_secret_store"),
        MicrosoftAccountServiceError::Auth(
            MicrosoftAuthError::Http { .. } | MicrosoftAuthError::HttpStatus { .. },
        ) => tr("app.skin.error_minecraft_unreachable"),
        other => other.to_string(),
    }
}

fn render_catalog_thumbnail(texture: &SkinTexture, variant: SkinVariant) -> Arc<RenderImage> {
    let renderer = SkinPreviewRenderer::new(180, 240);
    crate::app::ui::common::render_rgba_image(renderer.render(texture, variant, -0.45, 0.12, 0.0))
}

fn describe_catalog_error(error: &SkinCatalogError) -> String {
    match error {
        SkinCatalogError::HttpStatus { status } if status.as_u16() == 429 => {
            tr("app.skin.error_catalog_rate_limited")
        }
        SkinCatalogError::HttpStatus { status } => {
            rust_i18n::t!("app.skin.error_catalog_http", status = status).to_string()
        }
        SkinCatalogError::Http(_) | SkinCatalogError::Cancelled => {
            tr("app.skin.error_catalog_network")
        }
        SkinCatalogError::InvalidResponse(_) | SkinCatalogError::InvalidTextureHash => {
            tr("app.skin.error_catalog_response")
        }
        SkinCatalogError::ResponseTooLarge { .. } => tr("app.skin.error_catalog_too_large"),
        SkinCatalogError::Source(error) => match error {
            rmcl_app::SkinSourceError::UnsupportedUploadDimensions { width, height } => {
                rust_i18n::t!(
                    "app.skin.error_catalog_dimensions",
                    width = width,
                    height = height
                )
                .to_string()
            }
            rmcl_app::SkinSourceError::UploadTooLarge => {
                tr("app.skin.error_catalog_upload_too_large")
            }
            _ => tr("app.skin.error_catalog_texture"),
        },
        SkinCatalogError::InvalidUrl(_) => tr("app.skin.error_catalog_url"),
        SkinCatalogError::Blocking(_) => tr("app.skin.error_catalog_render"),
    }
}

fn describe_texture_error(error: &SkinError) -> String {
    match error {
        SkinError::HostNotAllowed | SkinError::InvalidUrl(_) | SkinError::UnsupportedScheme => {
            tr("app.skin.error_texture_url")
        }
        SkinError::Http { .. } | SkinError::HttpStatus { .. } | SkinError::TooLarge => {
            tr("app.skin.error_texture_download")
        }
        SkinError::Decode(_) | SkinError::InvalidDimensions { .. } => {
            tr("app.skin.error_texture_format")
        }
        _ => tr("app.skin.error_texture_process"),
    }
}

fn placeholder_skin() -> Option<SkinTexture> {
    let mut image = RgbaImage::from_pixel(64, 64, Rgba([150, 152, 156, 255]));
    for y in 0..16 {
        for x in 32..64 {
            image.put_pixel(x, y, Rgba([0, 0, 0, 0]));
        }
    }
    for y in 32..64 {
        for x in 0..64 {
            if (16..40).contains(&x) && (32..48).contains(&y)
                || (40..56).contains(&x) && (32..48).contains(&y)
                || (0..16).contains(&x) && (32..48).contains(&y)
                || (48..64).contains(&x) && (48..64).contains(&y)
                || (0..16).contains(&x) && (48..64).contains(&y)
            {
                image.put_pixel(x, y, Rgba([0, 0, 0, 0]));
            }
        }
    }
    for y in 8..16 {
        for x in 8..16 {
            image.put_pixel(x, y, Rgba([176, 178, 182, 255]));
        }
    }
    for (x, y) in [(9, 10), (10, 10), (13, 10), (14, 10)] {
        image.put_pixel(x, y, Rgba([70, 74, 80, 255]));
    }
    for x in 10..14 {
        image.put_pixel(x, 13, Rgba([110, 112, 116, 255]));
    }
    SkinTexture::from_rgba(image).ok()
}
