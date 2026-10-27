//! Shared official-skin profile and texture cache.
//!
//! Profiles are cached per account so leaving and re-entering the skin page
//! does not re-fetch them. Each mutually exclusive async operation lives in a
//! small state enum with its own task handle.

use std::{
    collections::HashMap,
    rc::Rc,
    time::{Duration, SystemTime},
};

use gpui_kit::*;
use image::{Rgba, RgbaImage};
use rmcl_app::{SkinError, SkinLibraryEntry, SkinTexture};
use rmcl_auth::{MicrosoftAccount, SkinVariant};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::services::Services;

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

pub enum SkinServiceEvent {
    ProfileLoaded(LoadedProfile),
}

impl EventEmitter<SkinServiceEvent> for SkinService {}

/// Per-account skin profiles, textures and the local recent list.
pub struct SkinService {
    services: Rc<Services>,
    profiles: HashMap<Uuid, ProfileState>,
    textures: HashMap<Uuid, EquippedSkinState>,
    recent: Loadable<Vec<SkinLibraryEntry>>,
    next_request: u64,
    recent_request: u64,
}

impl SkinService {
    pub fn new(services: Rc<Services>, _cx: &mut Context<Self>) -> Self {
        Self {
            services,
            profiles: HashMap::new(),
            textures: HashMap::new(),
            recent: Loadable::NotLoaded,
            next_request: 0,
            recent_request: 0,
        }
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
                                error: format!("皮肤资料请求失败: {error}"),
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
                        error: "无法生成默认皮肤预览。".to_owned(),
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
                                error: format!("皮肤纹理任务失败: {error}"),
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
                        error: format!("读取最近皮肤失败: {error}"),
                        previous,
                    },
                    Err(error) => Loadable::Failed {
                        error: format!("最近皮肤任务失败: {error}"),
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

fn describe_profile_error(error: &rmcl_app::MicrosoftAccountServiceError) -> String {
    use rmcl_app::MicrosoftAccountServiceError;
    use rmcl_auth::MicrosoftAuthError;

    match error {
        MicrosoftAccountServiceError::CredentialsMissing
        | MicrosoftAccountServiceError::Auth(
            MicrosoftAuthError::ReauthenticationRequired
            | MicrosoftAuthError::MinecraftTokenRejected,
        ) => "登录已过期，请重新登录。".to_owned(),
        MicrosoftAccountServiceError::SecretStore(_) => "系统凭据存储不可用。".to_owned(),
        MicrosoftAccountServiceError::Auth(
            MicrosoftAuthError::Http { .. } | MicrosoftAuthError::HttpStatus { .. },
        ) => "无法连接 Minecraft 服务，请稍后重试。".to_owned(),
        other => other.to_string(),
    }
}

fn describe_texture_error(error: &SkinError) -> String {
    match error {
        SkinError::HostNotAllowed | SkinError::InvalidUrl(_) | SkinError::UnsupportedScheme => {
            "官方皮肤地址无效。".to_owned()
        }
        SkinError::Http { .. } | SkinError::HttpStatus { .. } | SkinError::TooLarge => {
            "皮肤纹理下载失败。".to_owned()
        }
        SkinError::Decode(_) | SkinError::InvalidDimensions { .. } => {
            "皮肤纹理格式无效。".to_owned()
        }
        _ => "皮肤纹理处理失败。".to_owned(),
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
