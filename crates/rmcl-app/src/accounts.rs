//! Microsoft account use cases on top of the headless authentication protocol.

use std::sync::Arc;

use rmcl_auth::{
    AuthenticatedMicrosoft, MicrosoftAccount, MicrosoftAuth, MicrosoftAuthError,
    MicrosoftCredentials, SkinVariant,
};
use rmcl_launch::LaunchAccount;
use rmcl_platform::{SecretStore, SecretStoreError};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

/// Credential-store service name used by RMCL.
pub const MICROSOFT_KEYRING_SERVICE: &str = "com.rmcl.launcher";

/// A launch identity resolved from a Microsoft account.
#[derive(Debug, Clone)]
pub struct ResolvedMicrosoftAccount {
    /// Fresh profile metadata. The player name may differ from the cached one.
    pub profile: MicrosoftAccount,
    pub launch: LaunchAccount,
    /// Set when a rotated refresh token could not be persisted.
    pub persistence_warning: Option<String>,
}

/// Owns Microsoft authentication and encrypted credential persistence.
#[derive(Clone)]
pub struct MicrosoftAccountService {
    auth: MicrosoftAuth,
    secrets: Arc<dyn SecretStore>,
}

impl std::fmt::Debug for MicrosoftAccountService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MicrosoftAccountService")
            .finish_non_exhaustive()
    }
}

impl MicrosoftAccountService {
    pub fn new(secrets: Arc<dyn SecretStore>) -> Result<Self, MicrosoftAuthError> {
        Ok(Self {
            auth: MicrosoftAuth::new()?,
            secrets,
        })
    }

    /// Builds a service with explicit auth endpoints and secret storage.
    ///
    /// Intended for protocol tests and future self-hosted distributions.
    pub fn with_auth(auth: MicrosoftAuth, secrets: Arc<dyn SecretStore>) -> Self {
        Self { auth, secrets }
    }

    pub async fn start_device_login(&self) -> Result<rmcl_auth::DeviceLogin, MicrosoftAuthError> {
        self.auth.start_device_login().await
    }

    /// Persists a completed login. The account is unusable if this fails.
    pub async fn store_login(
        &self,
        authenticated: &AuthenticatedMicrosoft,
    ) -> Result<(), MicrosoftAccountServiceError> {
        self.save_credentials(&authenticated.account, &authenticated.credentials)
            .await
    }

    /// Loads and, when necessary, refreshes credentials for one account.
    ///
    /// Accounts without a stored credential entry must sign in again. A cached
    /// Minecraft token is used without contacting Microsoft.
    pub async fn resolve_launch_account(
        &self,
        account: &MicrosoftAccount,
        cancellation: &CancellationToken,
    ) -> Result<ResolvedMicrosoftAccount, MicrosoftAccountServiceError> {
        let Some(credentials) = self.load_credentials(account).await? else {
            return Err(MicrosoftAccountServiceError::CredentialsMissing);
        };
        if credentials.minecraft_token_valid() {
            return Ok(ResolvedMicrosoftAccount {
                profile: account.clone(),
                launch: LaunchAccount::from_microsoft(account, credentials.minecraft_access_token),
                persistence_warning: None,
            });
        }

        let authenticated = self
            .auth
            .refresh_session(&credentials, cancellation)
            .await?;
        let persistence_warning = match self
            .save_credentials(&authenticated.account, &authenticated.credentials)
            .await
        {
            Ok(()) => None,
            Err(error) => Some(format!(
                "无法保存刷新后的登录凭据，下次启动可能需要重新登录: {error}"
            )),
        };
        Ok(ResolvedMicrosoftAccount {
            profile: authenticated.account.clone(),
            launch: LaunchAccount::from_microsoft(
                &authenticated.account,
                authenticated.credentials.minecraft_access_token.clone(),
            ),
            persistence_warning,
        })
    }

    pub async fn remove_credentials(
        &self,
        account: &MicrosoftAccount,
    ) -> Result<(), MicrosoftAccountServiceError> {
        let secrets = self.secrets.clone();
        let key = credential_key(account);
        tokio::task::spawn_blocking(move || secrets.delete(&key))
            .await
            .map_err(|error| MicrosoftAccountServiceError::Blocking(error.to_string()))??;
        Ok(())
    }

    /// Fetches fresh profile metadata using cached credentials.
    ///
    /// Used when an account predates skin metadata, or when the caller needs a
    /// current player name. The cached Minecraft token is reused when valid.
    pub async fn refresh_profile(
        &self,
        account: &MicrosoftAccount,
        cancellation: &CancellationToken,
    ) -> Result<MicrosoftAccount, MicrosoftAccountServiceError> {
        let Some(credentials) = self.load_credentials(account).await? else {
            return Err(MicrosoftAccountServiceError::CredentialsMissing);
        };
        if credentials.minecraft_token_valid() {
            match self
                .auth
                .profile(&credentials.minecraft_access_token, cancellation)
                .await
            {
                Ok(profile) => return Ok(profile),
                Err(MicrosoftAuthError::MinecraftTokenRejected) => {}
                Err(error) => return Err(error.into()),
            }
        }
        let authenticated = self
            .auth
            .refresh_session(&credentials, cancellation)
            .await?;
        self.save_credentials(&authenticated.account, &authenticated.credentials)
            .await?;
        Ok(authenticated.account)
    }

    /// Uploads a validated PNG skin for one account.
    pub async fn upload_skin(
        &self,
        account: &MicrosoftAccount,
        variant: SkinVariant,
        file_name: &str,
        bytes: &[u8],
        cancellation: &CancellationToken,
    ) -> Result<MicrosoftAccount, MicrosoftAccountServiceError> {
        let mut credentials = self.credentials_for_skin(account, cancellation).await?;
        match self
            .auth
            .upload_skin(
                &credentials.minecraft_access_token,
                variant,
                file_name,
                bytes,
                cancellation,
            )
            .await
        {
            Ok(profile) => Ok(profile),
            Err(MicrosoftAuthError::MinecraftTokenRejected) => {
                credentials = self.refresh_credentials(&credentials, cancellation).await?;
                self.auth
                    .upload_skin(
                        &credentials.minecraft_access_token,
                        variant,
                        file_name,
                        bytes,
                        cancellation,
                    )
                    .await
                    .map_err(Into::into)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Changes the active skin using an image URL.
    pub async fn change_skin_url(
        &self,
        account: &MicrosoftAccount,
        variant: SkinVariant,
        url: &str,
        cancellation: &CancellationToken,
    ) -> Result<MicrosoftAccount, MicrosoftAccountServiceError> {
        let mut credentials = self.credentials_for_skin(account, cancellation).await?;
        match self
            .auth
            .change_skin_url(
                &credentials.minecraft_access_token,
                variant,
                url,
                cancellation,
            )
            .await
        {
            Ok(profile) => Ok(profile),
            Err(MicrosoftAuthError::MinecraftTokenRejected) => {
                credentials = self.refresh_credentials(&credentials, cancellation).await?;
                self.auth
                    .change_skin_url(
                        &credentials.minecraft_access_token,
                        variant,
                        url,
                        cancellation,
                    )
                    .await
                    .map_err(Into::into)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Resets the active skin to the account default.
    ///
    /// The returned profile is `None` when the service accepts the reset
    /// without returning profile data; callers should refresh in that case.
    pub async fn reset_skin(
        &self,
        account: &MicrosoftAccount,
        cancellation: &CancellationToken,
    ) -> Result<Option<MicrosoftAccount>, MicrosoftAccountServiceError> {
        let mut credentials = self.credentials_for_skin(account, cancellation).await?;
        match self
            .auth
            .reset_skin(&credentials.minecraft_access_token, cancellation)
            .await
        {
            Ok(profile) => Ok(profile),
            Err(MicrosoftAuthError::MinecraftTokenRejected) => {
                credentials = self.refresh_credentials(&credentials, cancellation).await?;
                self.auth
                    .reset_skin(&credentials.minecraft_access_token, cancellation)
                    .await
                    .map_err(Into::into)
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn credentials_for_skin(
        &self,
        account: &MicrosoftAccount,
        cancellation: &CancellationToken,
    ) -> Result<MicrosoftCredentials, MicrosoftAccountServiceError> {
        let credentials = self
            .load_credentials(account)
            .await?
            .ok_or(MicrosoftAccountServiceError::CredentialsMissing)?;
        if credentials.minecraft_token_valid() {
            Ok(credentials)
        } else {
            self.refresh_credentials(&credentials, cancellation).await
        }
    }

    async fn refresh_credentials(
        &self,
        credentials: &MicrosoftCredentials,
        cancellation: &CancellationToken,
    ) -> Result<MicrosoftCredentials, MicrosoftAccountServiceError> {
        let authenticated = self.auth.refresh_session(credentials, cancellation).await?;
        self.save_credentials(&authenticated.account, &authenticated.credentials)
            .await?;
        Ok(authenticated.credentials)
    }

    async fn save_credentials(
        &self,
        account: &MicrosoftAccount,
        credentials: &MicrosoftCredentials,
    ) -> Result<(), MicrosoftAccountServiceError> {
        if credentials.version != MicrosoftCredentials::SCHEMA_VERSION {
            return Err(MicrosoftAccountServiceError::UnsupportedCredentialsSchema {
                found: credentials.version,
            });
        }
        let key = credential_key(account);
        let value =
            serde_json::to_string(credentials).map_err(MicrosoftAccountServiceError::Serialize)?;
        let secrets = self.secrets.clone();
        tokio::task::spawn_blocking(move || secrets.set(&key, &value))
            .await
            .map_err(|error| MicrosoftAccountServiceError::Blocking(error.to_string()))??;
        Ok(())
    }

    async fn load_credentials(
        &self,
        account: &MicrosoftAccount,
    ) -> Result<Option<MicrosoftCredentials>, MicrosoftAccountServiceError> {
        let secrets = self.secrets.clone();
        let key = credential_key(account);
        let stored = tokio::task::spawn_blocking(move || secrets.get(&key))
            .await
            .map_err(|error| MicrosoftAccountServiceError::Blocking(error.to_string()))??;
        let Some(stored) = stored else {
            return Ok(None);
        };
        let credentials: MicrosoftCredentials =
            serde_json::from_str(&stored).map_err(MicrosoftAccountServiceError::Deserialize)?;
        if credentials.version != MicrosoftCredentials::SCHEMA_VERSION {
            return Err(MicrosoftAccountServiceError::UnsupportedCredentialsSchema {
                found: credentials.version,
            });
        }
        Ok(Some(credentials))
    }
}

fn credential_key(account: &MicrosoftAccount) -> String {
    format!("microsoft:{}", account.compact_uuid())
}

#[derive(Debug, Error)]
pub enum MicrosoftAccountServiceError {
    #[error(transparent)]
    Auth(#[from] MicrosoftAuthError),
    #[error(transparent)]
    SecretStore(#[from] SecretStoreError),
    #[error("no stored Microsoft credentials; sign in again")]
    CredentialsMissing,
    #[error("stored Microsoft credentials use unsupported schema {found}")]
    UnsupportedCredentialsSchema { found: u32 },
    #[error("failed to serialize Microsoft credentials: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("stored Microsoft credentials are invalid: {0}")]
    Deserialize(#[source] serde_json::Error),
    #[error("credential storage task failed: {0}")]
    Blocking(String),
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, StatusCode, header::AUTHORIZATION},
        response::{IntoResponse, Response},
        routing::{delete, get, post},
    };
    use rmcl_auth::MicrosoftEndpoints;
    use rmcl_platform::MemorySecretStore;
    use serde_json::{Value, json};
    use uuid::Uuid;

    use super::*;

    fn account() -> MicrosoftAccount {
        MicrosoftAccount::new(
            "HowDoesAuthWork",
            Uuid::parse_str("986dec87b7ec47ff89ff033fdb95c4b5").unwrap(),
        )
        .unwrap()
    }

    fn service(secrets: Arc<MemorySecretStore>) -> MicrosoftAccountService {
        let auth = MicrosoftAuth::with_endpoints(
            "test-client",
            MicrosoftEndpoints {
                device_code: "http://127.0.0.1:1/device".to_owned(),
                token: "http://127.0.0.1:1/token".to_owned(),
                xbox_authenticate: "http://127.0.0.1:1/xbox".to_owned(),
                xsts_authorize: "http://127.0.0.1:1/xsts".to_owned(),
                minecraft_login: "http://127.0.0.1:1/login".to_owned(),
                minecraft_entitlements: "http://127.0.0.1:1/entitlements".to_owned(),
                minecraft_profile: "http://127.0.0.1:1/profile".to_owned(),
                minecraft_profile_skins: "http://127.0.0.1:1/profile/skins".to_owned(),
                minecraft_profile_skins_active: "http://127.0.0.1:1/profile/skins/active"
                    .to_owned(),
            },
        )
        .unwrap();
        MicrosoftAccountService::with_auth(auth, secrets)
    }

    #[tokio::test]
    async fn cached_token_is_used_without_network_access() {
        let secrets = Arc::new(MemorySecretStore::default());
        let service = service(secrets.clone());
        let account = account();
        let credentials = MicrosoftCredentials::new("refresh", "minecraft-token", u64::MAX);
        service
            .save_credentials(&account, &credentials)
            .await
            .unwrap();

        let resolved = service
            .resolve_launch_account(&account, &CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(resolved.profile, account);
        assert_eq!(resolved.launch.access_token, "minecraft-token");
        assert!(resolved.persistence_warning.is_none());
        let raw = secrets.get(&credential_key(&account)).unwrap().unwrap();
        assert!(raw.contains("minecraft-token"));
    }

    #[tokio::test]
    async fn missing_credentials_require_sign_in() {
        let secrets = Arc::new(MemorySecretStore::default());
        let service = service(secrets);

        let error = service
            .resolve_launch_account(&account(), &CancellationToken::new())
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            MicrosoftAccountServiceError::CredentialsMissing
        ));
    }

    #[tokio::test]
    async fn stored_credentials_round_trip_and_are_removed() {
        let secrets = Arc::new(MemorySecretStore::default());
        let service = service(secrets.clone());
        let account = account();
        let authenticated = AuthenticatedMicrosoft {
            account: account.clone(),
            credentials: MicrosoftCredentials::new("refresh", "minecraft-token", u64::MAX),
        };

        service.store_login(&authenticated).await.unwrap();
        assert!(secrets.get(&credential_key(&account)).unwrap().is_some());
        service.remove_credentials(&account).await.unwrap();
        assert!(secrets.get(&credential_key(&account)).unwrap().is_none());
    }

    #[tokio::test]
    async fn unsupported_credentials_schema_is_rejected() {
        let secrets = Arc::new(MemorySecretStore::default());
        let service = service(secrets.clone());
        let account = account();
        let mut credentials = MicrosoftCredentials::new("refresh", "minecraft", u64::MAX);
        credentials.version = MicrosoftCredentials::SCHEMA_VERSION + 1;

        let error = service
            .save_credentials(&account, &credentials)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            MicrosoftAccountServiceError::UnsupportedCredentialsSchema { .. }
        ));
    }

    type RecordedSkinRequests = Arc<Mutex<Vec<(String, Option<String>)>>>;

    #[derive(Clone)]
    struct SkinMockState {
        requests: RecordedSkinRequests,
        skin_calls: Arc<AtomicUsize>,
        reject_first_skin: bool,
    }

    impl SkinMockState {
        fn new(reject_first_skin: bool) -> Self {
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                skin_calls: Arc::new(AtomicUsize::new(0)),
                reject_first_skin,
            }
        }
    }

    async fn spawn_skin_mock(state: SkinMockState) -> String {
        let router = Router::new()
            .route("/token", post(mock_token))
            .route("/xbox", post(mock_xbox))
            .route("/xsts", post(mock_xsts))
            .route("/login", post(mock_login))
            .route("/entitlements", get(mock_entitlements))
            .route("/profile", get(mock_profile))
            .route("/profile/skins", post(mock_skin))
            .route("/profile/skins/active", delete(mock_reset))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        format!("http://{address}")
    }

    fn mock_service(base: &str, secrets: Arc<MemorySecretStore>) -> MicrosoftAccountService {
        let auth = MicrosoftAuth::with_endpoints(
            "test-client",
            MicrosoftEndpoints {
                device_code: format!("{base}/device"),
                token: format!("{base}/token"),
                xbox_authenticate: format!("{base}/xbox"),
                xsts_authorize: format!("{base}/xsts"),
                minecraft_login: format!("{base}/login"),
                minecraft_entitlements: format!("{base}/entitlements"),
                minecraft_profile: format!("{base}/profile"),
                minecraft_profile_skins: format!("{base}/profile/skins"),
                minecraft_profile_skins_active: format!("{base}/profile/skins/active"),
            },
        )
        .unwrap();
        MicrosoftAccountService::with_auth(auth, secrets)
    }

    fn profile_json(active_url: &str, variant: &str) -> Value {
        json!({
            "id": "986dec87b7ec47ff89ff033fdb95c4b5",
            "name": "HowDoesAuthWork",
            "skins": [{
                "state": "ACTIVE",
                "url": active_url,
                "variant": variant
            }]
        })
    }

    fn record(state: &SkinMockState, path: &str, headers: &HeaderMap) {
        state.requests.lock().unwrap().push((
            path.to_owned(),
            headers
                .get(AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        ));
    }

    async fn mock_token(State(state): State<SkinMockState>, headers: HeaderMap) -> Response {
        record(&state, "/token", &headers);
        (
            StatusCode::OK,
            Json(json!({
                "access_token": "new-msa",
                "refresh_token": "new-refresh",
                "expires_in": 3600,
                "token_type": "Bearer"
            })),
        )
            .into_response()
    }

    async fn mock_xbox(State(state): State<SkinMockState>, headers: HeaderMap) -> Response {
        record(&state, "/xbox", &headers);
        (
            StatusCode::OK,
            Json(json!({
                "Token": "xbl-token",
                "DisplayClaims": {"xui": [{"uhs": "uhs-123"}]}
            })),
        )
            .into_response()
    }

    async fn mock_xsts(State(state): State<SkinMockState>, headers: HeaderMap) -> Response {
        record(&state, "/xsts", &headers);
        (
            StatusCode::OK,
            Json(json!({
                "Token": "xsts-token",
                "DisplayClaims": {"xui": [{"uhs": "uhs-123"}]}
            })),
        )
            .into_response()
    }

    async fn mock_login(State(state): State<SkinMockState>, headers: HeaderMap) -> Response {
        record(&state, "/login", &headers);
        (
            StatusCode::OK,
            Json(json!({
                "access_token": "new-minecraft",
                "expires_in": 86400,
                "token_type": "Bearer"
            })),
        )
            .into_response()
    }

    async fn mock_entitlements(State(state): State<SkinMockState>, headers: HeaderMap) -> Response {
        record(&state, "/entitlements", &headers);
        (
            StatusCode::OK,
            Json(json!({"items": [{"name": "game_minecraft"}]})),
        )
            .into_response()
    }

    async fn mock_profile(State(state): State<SkinMockState>, headers: HeaderMap) -> Response {
        record(&state, "/profile", &headers);
        (
            StatusCode::OK,
            Json(profile_json(
                "http://textures.minecraft.net/texture/current",
                "CLASSIC",
            )),
        )
            .into_response()
    }

    async fn mock_skin(State(state): State<SkinMockState>, headers: HeaderMap) -> Response {
        record(&state, "/profile/skins", &headers);
        let call = state.skin_calls.fetch_add(1, Ordering::SeqCst);
        if state.reject_first_skin && call == 0 {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        (
            StatusCode::OK,
            Json(profile_json(
                "http://textures.minecraft.net/texture/new",
                "SLIM",
            )),
        )
            .into_response()
    }

    async fn mock_reset(State(state): State<SkinMockState>, headers: HeaderMap) -> Response {
        record(&state, "/profile/skins/active", &headers);
        (
            StatusCode::OK,
            Json(profile_json(
                "http://textures.minecraft.net/texture/default",
                "CLASSIC",
            )),
        )
            .into_response()
    }

    async fn store_valid_credentials(
        service: &MicrosoftAccountService,
        account: &MicrosoftAccount,
    ) {
        service
            .save_credentials(
                account,
                &MicrosoftCredentials::new("refresh", "minecraft-token", u64::MAX),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn upload_skin_uses_cached_token() {
        let state = SkinMockState::new(false);
        let base = spawn_skin_mock(state.clone()).await;
        let secrets = Arc::new(MemorySecretStore::default());
        let service = mock_service(&base, secrets);
        let account = account();
        store_valid_credentials(&service, &account).await;

        let profile = service
            .upload_skin(
                &account,
                SkinVariant::Slim,
                "skin.png",
                b"png",
                &CancellationToken::new(),
            )
            .await
            .unwrap();

        assert_eq!(
            profile.skin_url(),
            Some("http://textures.minecraft.net/texture/new")
        );
        assert_eq!(state.skin_calls.load(Ordering::SeqCst), 1);
        let requests = state.requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1.as_deref(), Some("Bearer minecraft-token"));
    }

    #[tokio::test]
    async fn upload_skin_refreshes_once_after_token_rejection() {
        let state = SkinMockState::new(true);
        let base = spawn_skin_mock(state.clone()).await;
        let secrets = Arc::new(MemorySecretStore::default());
        let service = mock_service(&base, secrets.clone());
        let account = account();
        store_valid_credentials(&service, &account).await;

        let profile = service
            .upload_skin(
                &account,
                SkinVariant::Slim,
                "skin.png",
                b"png",
                &CancellationToken::new(),
            )
            .await
            .unwrap();

        assert_eq!(
            profile.skin_url(),
            Some("http://textures.minecraft.net/texture/new")
        );
        assert_eq!(state.skin_calls.load(Ordering::SeqCst), 2);
        let requests = state.requests.lock().unwrap().clone();
        let skin_auth = requests
            .iter()
            .filter(|(path, _)| path == "/profile/skins")
            .map(|(_, auth)| auth.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(
            skin_auth,
            vec![Some("Bearer minecraft-token"), Some("Bearer new-minecraft")]
        );
        let raw = secrets.get(&credential_key(&account)).unwrap().unwrap();
        assert!(raw.contains("new-refresh"));
        assert!(raw.contains("new-minecraft"));
    }

    #[tokio::test]
    async fn reset_skin_uses_cached_token() {
        let state = SkinMockState::new(false);
        let base = spawn_skin_mock(state.clone()).await;
        let secrets = Arc::new(MemorySecretStore::default());
        let service = mock_service(&base, secrets);
        let account = account();
        store_valid_credentials(&service, &account).await;

        let profile = service
            .reset_skin(&account, &CancellationToken::new())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            profile.skin_url(),
            Some("http://textures.minecraft.net/texture/default")
        );
        let requests = state.requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, "/profile/skins/active");
        assert_eq!(requests[0].1.as_deref(), Some("Bearer minecraft-token"));
    }
}
