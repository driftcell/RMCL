//! Third-party Yggdrasil account use cases and credential persistence.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use rmcl_auth::{
    AuthlibAccount, AuthlibAccountError, AuthlibClient, AuthlibCredentials, AuthlibError,
    AuthlibLogin, AuthlibProfile, AuthlibServer,
};
use rmcl_launch::LaunchAccount;
use rmcl_platform::{SecretStore, SecretStoreError};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

use crate::MICROSOFT_KEYRING_SERVICE;

/// A launch identity resolved from a third-party Yggdrasil account.
#[derive(Debug, Clone)]
pub struct ResolvedAuthlibAccount {
    /// Profile metadata that may have changed after refresh.
    pub account: AuthlibAccount,
    pub launch: LaunchAccount,
    pub server: AuthlibServer,
    /// Base64 of the exact API root metadata response for authlib-injector.
    /// `None` when the metadata is too large for a portable command line.
    pub prefetched_metadata: Option<String>,
    /// Set when a rotated token could not be persisted.
    pub persistence_warning: Option<String>,
}

/// Owns third-party Yggdrasil authentication and credential persistence.
#[derive(Clone)]
pub struct AuthlibAccountService {
    auth: AuthlibClient,
    secrets: Arc<dyn SecretStore>,
    metadata: AuthlibMetadataCache,
}

impl std::fmt::Debug for AuthlibAccountService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthlibAccountService")
            .finish_non_exhaustive()
    }
}

impl AuthlibAccountService {
    pub fn new(
        secrets: Arc<dyn SecretStore>,
        metadata_cache_dir: impl Into<PathBuf>,
    ) -> Result<Self, AuthlibAccountServiceError> {
        Ok(Self {
            auth: AuthlibClient::new()?,
            secrets,
            metadata: AuthlibMetadataCache::new(metadata_cache_dir),
        })
    }

    /// Builds a service with an explicit protocol client for tests.
    pub fn with_auth(
        auth: AuthlibClient,
        secrets: Arc<dyn SecretStore>,
        metadata_cache_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            auth,
            secrets,
            metadata: AuthlibMetadataCache::new(metadata_cache_dir),
        }
    }

    pub async fn authenticate(
        &self,
        server_input: &str,
        username: &str,
        password: &str,
        cancellation: &CancellationToken,
    ) -> Result<AuthlibLogin, AuthlibAccountServiceError> {
        let server = self.auth.resolve_server(server_input, cancellation).await?;
        let login = self
            .auth
            .authenticate(&server, username, password, Uuid::new_v4(), cancellation)
            .await?;
        let _ = self.metadata.store(&server).await;
        Ok(login)
    }

    /// Completes a login with a profile chosen by the player.
    pub async fn complete_login(
        &self,
        login: AuthlibLogin,
        profile: &AuthlibProfile,
        cancellation: &CancellationToken,
    ) -> Result<AuthlibAccount, AuthlibAccountServiceError> {
        self.complete_login_with_id(login, profile, Uuid::new_v4(), cancellation)
            .await
    }

    /// Completes a login while preserving an existing launcher account ID.
    pub async fn complete_login_with_id(
        &self,
        login: AuthlibLogin,
        profile: &AuthlibProfile,
        account_id: Uuid,
        cancellation: &CancellationToken,
    ) -> Result<AuthlibAccount, AuthlibAccountServiceError> {
        let (login, profile) = if login
            .selected_profile()
            .is_some_and(|selected| selected.id() == profile.id())
        {
            (login, profile.clone())
        } else {
            let refreshed = self
                .auth
                .refresh(
                    login.server(),
                    login.credentials(),
                    Some(profile),
                    cancellation,
                )
                .await?;
            let selected = refreshed
                .selected_profile()
                .cloned()
                .ok_or(AuthlibError::ProfileMismatch)?;
            if selected.id() != profile.id() {
                return Err(AuthlibError::ProfileMismatch.into());
            }
            (refreshed, selected)
        };

        let account = AuthlibAccount::from_profile(account_id, &profile, login.server())?;
        self.save_credentials(&account, login.credentials()).await?;
        let _ = self.metadata.store(login.server()).await;
        Ok(account)
    }

    /// Completes a login when the server selected a profile or only one exists.
    pub async fn complete_login_default(
        &self,
        login: AuthlibLogin,
        cancellation: &CancellationToken,
    ) -> Result<AuthlibAccount, AuthlibAccountServiceError> {
        let profile = login
            .single_profile()
            .cloned()
            .ok_or(AuthlibAccountServiceError::ProfileSelectionRequired)?;
        self.complete_login(login, &profile, cancellation).await
    }

    /// Loads and, when necessary, refreshes credentials for one account.
    ///
    /// A cached token is validated against the authentication server. A
    /// rejected token is refreshed; a rejected refresh requires signing in
    /// again and never silently falls back to an offline identity.
    pub async fn resolve_launch_account(
        &self,
        account: &AuthlibAccount,
        cancellation: &CancellationToken,
    ) -> Result<ResolvedAuthlibAccount, AuthlibAccountServiceError> {
        let credentials = self
            .load_credentials(account)
            .await?
            .ok_or(AuthlibAccountServiceError::CredentialsMissing)?;
        let server = match self
            .auth
            .resolve_server(account.api_root(), cancellation)
            .await
        {
            Ok(server) => {
                let _ = self.metadata.store(&server).await;
                server
            }
            Err(error) => self.metadata.load(account.api_root()).await.ok_or(error)?,
        };

        let (credentials, refreshed_profile, persistence_warning) = if self
            .auth
            .validate(&server, &credentials, cancellation)
            .await?
        {
            (credentials, None, None)
        } else {
            let expected = AuthlibProfile::new(account.uuid(), account.username())?;
            let login = self
                .auth
                .refresh(&server, &credentials, Some(&expected), cancellation)
                .await?;
            let selected = login
                .selected_profile()
                .cloned()
                .ok_or(AuthlibError::ProfileMismatch)?;
            if selected.id() != account.uuid() {
                return Err(AuthlibError::ProfileMismatch.into());
            }
            let persistence_warning =
                match self.save_credentials(account, login.credentials()).await {
                    Ok(()) => None,
                    Err(error) => Some(format!(
                        "无法保存刷新后的外置登录凭据，下次启动可能需要重新登录: {error}"
                    )),
                };
            (
                login.credentials().clone(),
                Some(selected),
                persistence_warning,
            )
        };

        let mut account = account.clone();
        if let Some(profile) = refreshed_profile {
            account.update_profile(&profile, &server);
        }
        let launch = LaunchAccount::from_authlib(
            &account,
            credentials.access_token.clone(),
            credentials.user_properties.clone(),
        );
        let prefetched = BASE64.encode(server.metadata_raw().as_bytes());
        // Keep the Windows command line below its 32 KiB limit. authlib-injector
        // can fetch the metadata itself when prefetch is omitted.
        let prefetched_metadata = (prefetched.len() <= 24 * 1024).then_some(prefetched);
        Ok(ResolvedAuthlibAccount {
            account,
            launch,
            prefetched_metadata,
            server,
            persistence_warning,
        })
    }

    pub async fn remove_credentials(
        &self,
        account: &AuthlibAccount,
    ) -> Result<(), AuthlibAccountServiceError> {
        let secrets = self.secrets.clone();
        let key = credential_key(account);
        tokio::task::spawn_blocking(move || secrets.delete(&key))
            .await
            .map_err(|error| AuthlibAccountServiceError::Blocking(error.to_string()))??;
        Ok(())
    }

    async fn save_credentials(
        &self,
        account: &AuthlibAccount,
        credentials: &AuthlibCredentials,
    ) -> Result<(), AuthlibAccountServiceError> {
        if credentials.version != AuthlibCredentials::SCHEMA_VERSION {
            return Err(AuthlibAccountServiceError::UnsupportedCredentialsSchema {
                found: credentials.version,
            });
        }
        let key = credential_key(account);
        let value =
            serde_json::to_string(credentials).map_err(AuthlibAccountServiceError::Serialize)?;
        let secrets = self.secrets.clone();
        tokio::task::spawn_blocking(move || secrets.set(&key, &value))
            .await
            .map_err(|error| AuthlibAccountServiceError::Blocking(error.to_string()))??;
        Ok(())
    }

    async fn load_credentials(
        &self,
        account: &AuthlibAccount,
    ) -> Result<Option<AuthlibCredentials>, AuthlibAccountServiceError> {
        let secrets = self.secrets.clone();
        let key = credential_key(account);
        let stored = tokio::task::spawn_blocking(move || secrets.get(&key))
            .await
            .map_err(|error| AuthlibAccountServiceError::Blocking(error.to_string()))??;
        let Some(stored) = stored else {
            return Ok(None);
        };
        let credentials: AuthlibCredentials =
            serde_json::from_str(&stored).map_err(AuthlibAccountServiceError::Deserialize)?;
        if credentials.version != AuthlibCredentials::SCHEMA_VERSION {
            return Err(AuthlibAccountServiceError::UnsupportedCredentialsSchema {
                found: credentials.version,
            });
        }
        Ok(Some(credentials))
    }
}

fn credential_key(account: &AuthlibAccount) -> String {
    format!("authlib:{}", account.id())
}

#[derive(Debug, Clone)]
struct AuthlibMetadataCache {
    root: PathBuf,
}

impl AuthlibMetadataCache {
    fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    async fn load(&self, api_root: &str) -> Option<AuthlibServer> {
        let api_root = Url::parse(api_root).ok()?;
        let path = self.path(api_root.as_str());
        let raw = tokio::fs::read_to_string(path).await.ok()?;
        AuthlibServer::from_metadata(api_root, raw).ok()
    }

    async fn store(&self, server: &AuthlibServer) -> Result<(), AuthlibAccountServiceError> {
        let path = self.path(server.api_root());
        write_atomic(&path, server.metadata_raw().as_bytes()).await
    }

    fn path(&self, api_root: &str) -> PathBuf {
        let hash = hex::encode(Sha256::digest(api_root.as_bytes()));
        self.root.join(format!("{hash}.json"))
    }
}

async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), AuthlibAccountServiceError> {
    let path = path.to_path_buf();
    let parent =
        path.parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| AuthlibAccountServiceError::Cache {
                path: path.display().to_string(),
                message: "cache path has no parent".to_owned(),
            })?;
    tokio::fs::create_dir_all(&parent)
        .await
        .map_err(|source| AuthlibAccountServiceError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    let bytes = bytes.to_vec();
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let mut temporary = tempfile::NamedTempFile::new_in(&parent).map_err(|source| {
            AuthlibAccountServiceError::Io {
                path: path.display().to_string(),
                source,
            }
        })?;
        temporary
            .write_all(&bytes)
            .map_err(|source| AuthlibAccountServiceError::Io {
                path: temporary.path().display().to_string(),
                source,
            })?;
        temporary
            .persist(&path)
            .map_err(|error| AuthlibAccountServiceError::Io {
                path: path.display().to_string(),
                source: error.error,
            })?;
        Ok::<_, AuthlibAccountServiceError>(())
    })
    .await
    .map_err(|error| AuthlibAccountServiceError::Blocking(error.to_string()))?
}

#[derive(Debug, Error)]
pub enum AuthlibAccountServiceError {
    #[error(transparent)]
    Auth(#[from] AuthlibError),
    #[error(transparent)]
    Account(#[from] AuthlibAccountError),
    #[error(transparent)]
    SecretStore(#[from] SecretStoreError),
    #[error("no stored third-party credentials; sign in again")]
    CredentialsMissing,
    #[error("the account owns multiple profiles; choose one before signing in")]
    ProfileSelectionRequired,
    #[error("stored third-party credentials use unsupported schema {found}")]
    UnsupportedCredentialsSchema { found: u32 },
    #[error("failed to serialize third-party credentials: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("stored third-party credentials are invalid: {0}")]
    Deserialize(#[source] serde_json::Error),
    #[error("credential storage task failed: {0}")]
    Blocking(String),
    #[error("metadata cache I/O failed at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("metadata cache failed at {path}: {message}")]
    Cache { path: String, message: String },
}

/// The keyring service name shared with Microsoft and agent credentials.
pub const AUTHLIB_KEYRING_SERVICE: &str = MICROSOFT_KEYRING_SERVICE;

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use axum::{
        Json, Router,
        extract::State,
        http::StatusCode,
        response::IntoResponse,
        routing::{get, post},
    };
    use rmcl_platform::MemorySecretStore;
    use serde_json::{Value, json};

    use super::*;

    #[derive(Clone, Default)]
    struct MockState {
        auth_calls: Arc<AtomicUsize>,
        refresh_calls: Arc<AtomicUsize>,
        validate_calls: Arc<AtomicUsize>,
        validate_ok: Arc<std::sync::atomic::AtomicBool>,
        multiple_profiles: Arc<std::sync::atomic::AtomicBool>,
    }

    async fn spawn_mock() -> (String, MockState) {
        let state = MockState::default();
        state
            .validate_ok
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let router = Router::new()
            .route("/api/yggdrasil/", get(mock_metadata))
            .route(
                "/api/yggdrasil/authserver/authenticate",
                post(mock_authenticate),
            )
            .route("/api/yggdrasil/authserver/refresh", post(mock_refresh))
            .route("/api/yggdrasil/authserver/validate", post(mock_validate))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (format!("http://{address}/api/yggdrasil/"), state)
    }

    async fn mock_metadata() -> impl IntoResponse {
        Json(json!({
            "meta": {"serverName": "Mock Skin"},
            "skinDomains": ["example.invalid"]
        }))
    }

    async fn mock_authenticate(
        State(state): State<MockState>,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        state.auth_calls.fetch_add(1, Ordering::SeqCst);
        let client_token = body["clientToken"].as_str().unwrap().to_owned();
        let multiple = state
            .multiple_profiles
            .load(std::sync::atomic::Ordering::SeqCst);
        let selected = if multiple {
            Value::Null
        } else {
            json!({"id": "5627dd98e6be3c21b8a8e92344183641", "name": "Steve"})
        };
        Json(json!({
            "accessToken": "access-token",
            "clientToken": client_token,
            "selectedProfile": selected,
            "availableProfiles": [
                {"id": "5627dd98e6be3c21b8a8e92344183641", "name": "Steve"},
                {"id": "36532b5ec4423dbba24cc7e55d0f979a", "name": "Alex"}
            ],
            "user": {"id": "user-1", "properties": []}
        }))
        .into_response()
    }

    async fn mock_refresh(
        State(state): State<MockState>,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        state.refresh_calls.fetch_add(1, Ordering::SeqCst);
        let client_token = body["clientToken"].as_str().unwrap().to_owned();
        let selected = body
            .get("selectedProfile")
            .cloned()
            .unwrap_or_else(|| json!({"id": "5627dd98e6be3c21b8a8e92344183641", "name": "Steve"}));
        Json(json!({
            "accessToken": "refreshed-token",
            "clientToken": client_token,
            "selectedProfile": selected,
            "availableProfiles": [selected],
            "user": {"id": "user-1", "properties": []}
        }))
        .into_response()
    }

    async fn mock_validate(State(state): State<MockState>) -> impl IntoResponse {
        state.validate_calls.fetch_add(1, Ordering::SeqCst);
        if state.validate_ok.load(std::sync::atomic::Ordering::SeqCst) {
            StatusCode::NO_CONTENT
        } else {
            StatusCode::FORBIDDEN
        }
    }

    fn service(_base: &str, secrets: Arc<MemorySecretStore>) -> AuthlibAccountService {
        let cache = tempfile::tempdir().unwrap();
        // Keep the temporary directory alive for the test process by leaking it;
        // metadata cache writes are best-effort.
        let cache = Box::leak(Box::new(cache));
        AuthlibAccountService::with_auth(AuthlibClient::new().unwrap(), secrets, cache.path())
    }

    #[tokio::test]
    async fn login_stores_credentials_and_resolves_a_launch_account() {
        let (base, state) = spawn_mock().await;
        let secrets = Arc::new(MemorySecretStore::default());
        let service = service(&base, secrets.clone());
        let cancellation = CancellationToken::new();

        let login = service
            .authenticate(&base, "Steve", "correct", &cancellation)
            .await
            .unwrap();
        let account = service
            .complete_login_default(login, &cancellation)
            .await
            .unwrap();
        assert_eq!(account.username(), "Steve");
        assert!(secrets.get(&credential_key(&account)).unwrap().is_some());

        let resolved = service
            .resolve_launch_account(&account, &cancellation)
            .await
            .unwrap();
        assert_eq!(resolved.launch.access_token, "access-token");
        assert_eq!(resolved.launch.user_type, "msa");
        assert_eq!(resolved.server.display_name(), "Mock Skin");
        assert_eq!(state.auth_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.validate_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.refresh_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn rejected_token_is_refreshed_and_persisted() {
        let (base, state) = spawn_mock().await;
        state
            .validate_ok
            .store(false, std::sync::atomic::Ordering::SeqCst);
        let secrets = Arc::new(MemorySecretStore::default());
        let service = service(&base, secrets.clone());
        let cancellation = CancellationToken::new();

        let login = service
            .authenticate(&base, "Steve", "correct", &cancellation)
            .await
            .unwrap();
        let account = service
            .complete_login_default(login, &cancellation)
            .await
            .unwrap();
        let resolved = service
            .resolve_launch_account(&account, &cancellation)
            .await
            .unwrap();
        assert_eq!(resolved.launch.access_token, "refreshed-token");
        assert_eq!(state.refresh_calls.load(Ordering::SeqCst), 1);
        let stored = secrets.get(&credential_key(&account)).unwrap().unwrap();
        assert!(stored.contains("refreshed-token"));
    }

    #[tokio::test]
    async fn multiple_profiles_require_a_selection() {
        let (base, state) = spawn_mock().await;
        state
            .multiple_profiles
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let secrets = Arc::new(MemorySecretStore::default());
        let service = service(&base, secrets);
        let cancellation = CancellationToken::new();

        let login = service
            .authenticate(&base, "Steve", "correct", &cancellation)
            .await
            .unwrap();
        assert!(matches!(
            service
                .complete_login_default(login.clone(), &cancellation)
                .await,
            Err(AuthlibAccountServiceError::ProfileSelectionRequired)
        ));

        let alex = login.available_profiles()[1].clone();
        let account = service
            .complete_login(login, &alex, &cancellation)
            .await
            .unwrap();
        assert_eq!(account.username(), "Alex");
        assert_eq!(state.refresh_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn metadata_cache_round_trips_raw_response() {
        let cache_dir = tempfile::tempdir().unwrap();
        let cache = AuthlibMetadataCache::new(cache_dir.path());
        let raw = r#"{"meta":{"serverName":"Cached"},"skinDomains":["example.invalid"]}"#;
        let server = AuthlibServer::from_metadata(
            Url::parse("https://example.invalid/api/yggdrasil/").unwrap(),
            raw.to_owned(),
        )
        .unwrap();

        cache.store(&server).await.unwrap();
        let loaded = cache.load(server.api_root()).await.unwrap();

        assert_eq!(loaded.metadata_raw(), raw);
        assert_eq!(loaded.display_name(), "Cached");
    }

    #[tokio::test]
    async fn missing_credentials_require_sign_in() {
        let (base, _) = spawn_mock().await;
        let secrets = Arc::new(MemorySecretStore::default());
        let service = service(&base, secrets);
        let account = AuthlibAccount::new(
            Uuid::new_v4(),
            "Steve",
            Uuid::parse_str("5627dd98e6be3c21b8a8e92344183641").unwrap(),
            base,
            Some("Mock Skin".to_owned()),
        )
        .unwrap();

        assert!(matches!(
            service
                .resolve_launch_account(&account, &CancellationToken::new())
                .await,
            Err(AuthlibAccountServiceError::CredentialsMissing)
        ));
    }
}
