//! Microsoft account authorization for Minecraft: Java Edition.
//!
//! The flow uses the OAuth 2.0 device code grant, then exchanges the Microsoft
//! token for Xbox Live, XSTS and Minecraft services tokens. Network endpoints
//! and the client ID are injectable so the protocol can be tested against a
//! local server.

use std::{
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use reqwest::{Client, RequestBuilder, StatusCode};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Public Xbox Live client used by the device code flow.
///
/// The device code grant is handled by `login.live.com`; this client does not
/// require an Azure application registration or a client secret.
pub const DEFAULT_MICROSOFT_CLIENT_ID: &str = "00000000402b5328";

/// Scopes required to exchange a Microsoft token for Xbox Live credentials.
pub const MICROSOFT_SCOPE: &str = "XboxLive.signin offline_access";

/// Refresh a cached Minecraft token this many seconds before it expires.
pub const MINECRAFT_TOKEN_REFRESH_MARGIN_SECS: u64 = 300;

const DEVICE_CODE_OPERATION: &str = "request Microsoft device code";
const POLL_OPERATION: &str = "poll Microsoft device code";
const REFRESH_OPERATION: &str = "refresh Microsoft token";
const XBOX_OPERATION: &str = "authenticate with Xbox Live";
const XSTS_OPERATION: &str = "authorize with Xbox Live";
const MINECRAFT_LOGIN_OPERATION: &str = "log in to Minecraft services";
const ENTITLEMENTS_OPERATION: &str = "check Minecraft ownership";
const PROFILE_OPERATION: &str = "fetch Minecraft profile";
const SKIN_UPLOAD_OPERATION: &str = "upload Minecraft skin";
const SKIN_CHANGE_OPERATION: &str = "change Minecraft skin";
const SKIN_RESET_OPERATION: &str = "reset Minecraft skin";

/// Endpoints used by [`MicrosoftAuth`].
///
/// Production code should use [`Self::default`]. Tests replace these with a
/// local mock server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicrosoftEndpoints {
    pub device_code: String,
    pub token: String,
    pub xbox_authenticate: String,
    pub xsts_authorize: String,
    pub minecraft_login: String,
    pub minecraft_entitlements: String,
    pub minecraft_profile: String,
    pub minecraft_profile_skins: String,
    pub minecraft_profile_skins_active: String,
}

impl Default for MicrosoftEndpoints {
    fn default() -> Self {
        Self {
            device_code: "https://login.live.com/oauth20_connect.srf".to_owned(),
            token: "https://login.live.com/oauth20_token.srf".to_owned(),
            xbox_authenticate: "https://user.auth.xboxlive.com/user/authenticate".to_owned(),
            xsts_authorize: "https://xsts.auth.xboxlive.com/xsts/authorize".to_owned(),
            minecraft_login: "https://api.minecraftservices.com/authentication/login_with_xbox"
                .to_owned(),
            minecraft_entitlements: "https://api.minecraftservices.com/entitlements/mcstore"
                .to_owned(),
            minecraft_profile: "https://api.minecraftservices.com/minecraft/profile".to_owned(),
            minecraft_profile_skins: "https://api.minecraftservices.com/minecraft/profile/skins"
                .to_owned(),
            minecraft_profile_skins_active:
                "https://api.minecraftservices.com/minecraft/profile/skins/active".to_owned(),
        }
    }
}

/// A Microsoft/Xbox/Minecraft authentication client.
#[derive(Clone)]
pub struct MicrosoftAuth {
    client: Client,
    client_id: String,
    endpoints: MicrosoftEndpoints,
}

impl fmt::Debug for MicrosoftAuth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MicrosoftAuth")
            .field("client_id", &self.client_id)
            .field("endpoints", &self.endpoints)
            .finish_non_exhaustive()
    }
}

impl MicrosoftAuth {
    /// Creates a client for the public Xbox Live device code flow.
    pub fn new() -> Result<Self, MicrosoftAuthError> {
        Self::with_endpoints(DEFAULT_MICROSOFT_CLIENT_ID, MicrosoftEndpoints::default())
    }

    /// Creates a client with explicit credentials and endpoints.
    ///
    /// This is primarily intended for local protocol tests and for builds that
    /// later provide their own registered OAuth application.
    pub fn with_endpoints(
        client_id: impl Into<String>,
        endpoints: MicrosoftEndpoints,
    ) -> Result<Self, MicrosoftAuthError> {
        let client_id = client_id.into();
        if client_id.trim().is_empty() {
            return Err(MicrosoftAuthError::InvalidClientId);
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("RMCL/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(MicrosoftAuthError::Client)?;
        Ok(Self {
            client,
            client_id,
            endpoints,
        })
    }

    /// Starts a device code flow.
    ///
    /// The returned flow exposes the one-time code and can be awaited with
    /// [`DeviceLogin::wait`].
    pub async fn start_device_login(&self) -> Result<DeviceLogin, MicrosoftAuthError> {
        let device_code = self.request_device_code().await?;
        Ok(DeviceLogin::new(self.clone(), device_code))
    }

    /// Requests a new device code.
    pub async fn request_device_code(&self) -> Result<DeviceCode, MicrosoftAuthError> {
        let request = self.client.post(&self.endpoints.device_code).form(&[
            ("client_id", self.client_id.as_str()),
            ("scope", MICROSOFT_SCOPE),
            ("response_type", "device_code"),
        ]);
        let (status, body) = self.execute(DEVICE_CODE_OPERATION, request).await?;
        if !status.is_success() {
            return Err(oauth_failure(DEVICE_CODE_OPERATION, status, &body));
        }
        let raw: RawDeviceCode = parse_json(DEVICE_CODE_OPERATION, &body)?;
        if raw.device_code.trim().is_empty()
            || raw.user_code.trim().is_empty()
            || raw.verification_uri.trim().is_empty()
        {
            return Err(MicrosoftAuthError::InvalidResponse(DEVICE_CODE_OPERATION));
        }
        Ok(DeviceCode {
            device_code: raw.device_code,
            user_code: raw.user_code,
            verification_uri: raw.verification_uri,
            expires_in: raw.expires_in,
            interval: raw.interval.unwrap_or(5).max(1),
        })
    }

    /// Polls the token endpoint once for a pending device code.
    pub async fn poll_device_code(
        &self,
        device_code: &str,
    ) -> Result<DeviceCodePoll, MicrosoftAuthError> {
        let request = self.client.post(&self.endpoints.token).form(&[
            ("client_id", self.client_id.as_str()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", device_code),
        ]);
        let (status, body) = self.execute(POLL_OPERATION, request).await?;
        if !status.is_success() {
            let error: RawOAuthError = match serde_json::from_str(&body) {
                Ok(error) => error,
                Err(_) => return Err(http_failure(POLL_OPERATION, status, &body)),
            };
            return match error.error.as_str() {
                "authorization_pending" => Ok(DeviceCodePoll::Pending),
                "slow_down" => Ok(DeviceCodePoll::SlowDown),
                "expired_token" => Err(MicrosoftAuthError::DeviceCodeExpired),
                "authorization_declined" | "access_denied" => {
                    Err(MicrosoftAuthError::DeviceCodeDeclined)
                }
                _ => Err(MicrosoftAuthError::OAuth {
                    code: error.error,
                    message: error.error_description.unwrap_or_default(),
                }),
            };
        }
        let tokens = parse_oauth_tokens(POLL_OPERATION, &body, None)?;
        Ok(DeviceCodePoll::Complete(tokens))
    }

    /// Exchanges a refresh token for a new Microsoft token pair.
    pub async fn refresh_oauth(
        &self,
        refresh_token: &str,
    ) -> Result<MicrosoftOAuthTokens, MicrosoftAuthError> {
        let request = self.client.post(&self.endpoints.token).form(&[
            ("client_id", self.client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("scope", MICROSOFT_SCOPE),
        ]);
        let (status, body) = self.execute(REFRESH_OPERATION, request).await?;
        if !status.is_success() {
            let error: RawOAuthError = match serde_json::from_str(&body) {
                Ok(error) => error,
                Err(_) => return Err(http_failure(REFRESH_OPERATION, status, &body)),
            };
            return match error.error.as_str() {
                "invalid_grant" | "interaction_required" | "consent_required" => {
                    Err(MicrosoftAuthError::ReauthenticationRequired)
                }
                _ => Err(MicrosoftAuthError::OAuth {
                    code: error.error,
                    message: error.error_description.unwrap_or_default(),
                }),
            };
        }
        parse_oauth_tokens(REFRESH_OPERATION, &body, Some(refresh_token))
    }

    /// Authenticates a Microsoft token with Xbox Live.
    pub async fn authenticate_xbox(
        &self,
        microsoft_access_token: &str,
        cancellation: &CancellationToken,
    ) -> Result<XboxToken, MicrosoftAuthError> {
        check_cancelled(cancellation)?;
        let request = self
            .client
            .post(&self.endpoints.xbox_authenticate)
            .header("Accept", "application/json")
            .json(&XboxUserAuthRequest {
                properties: XboxUserAuthProperties {
                    auth_method: "RPS",
                    site_name: "user.auth.xboxlive.com",
                    rps_ticket: format!("d={microsoft_access_token}"),
                },
                relying_party: "http://auth.xboxlive.com",
                token_type: "JWT",
            });
        let (status, body) = self.execute(XBOX_OPERATION, request).await?;
        if !status.is_success() {
            return Err(http_failure(XBOX_OPERATION, status, &body));
        }
        parse_xbox_token(XBOX_OPERATION, &body)
    }

    /// Exchanges an Xbox Live token for an XSTS token scoped to Minecraft.
    pub async fn authorize_xsts(
        &self,
        xbox_token: &str,
        cancellation: &CancellationToken,
    ) -> Result<XboxToken, MicrosoftAuthError> {
        check_cancelled(cancellation)?;
        let request = self
            .client
            .post(&self.endpoints.xsts_authorize)
            .header("Accept", "application/json")
            .json(&XstsRequest {
                properties: XstsProperties {
                    sandbox_id: "RETAIL",
                    user_tokens: [xbox_token],
                },
                relying_party: "rp://api.minecraftservices.com/",
                token_type: "JWT",
            });
        let (status, body) = self.execute(XSTS_OPERATION, request).await?;
        if status == StatusCode::UNAUTHORIZED
            && let Ok(error) = serde_json::from_str::<RawXstsError>(&body)
            && error.xerr.is_some()
        {
            return Err(MicrosoftAuthError::Xbox {
                code: error.xerr,
                message: error.message.unwrap_or_default(),
                redirect: error.redirect,
            });
        }
        if !status.is_success() {
            return Err(http_failure(XSTS_OPERATION, status, &body));
        }
        parse_xbox_token(XSTS_OPERATION, &body)
    }

    /// Logs in to Minecraft services with an XSTS token.
    pub async fn login_minecraft(
        &self,
        user_hash: &str,
        xsts_token: &str,
        cancellation: &CancellationToken,
    ) -> Result<MinecraftToken, MicrosoftAuthError> {
        check_cancelled(cancellation)?;
        let request = self
            .client
            .post(&self.endpoints.minecraft_login)
            .header("Accept", "application/json")
            .json(&MinecraftLoginRequest {
                identity_token: format!("XBL3.0 x={user_hash};{xsts_token}"),
            });
        let (status, body) = self.execute(MINECRAFT_LOGIN_OPERATION, request).await?;
        if !status.is_success() {
            return Err(http_failure(MINECRAFT_LOGIN_OPERATION, status, &body));
        }
        let raw: RawMinecraftToken = parse_json(MINECRAFT_LOGIN_OPERATION, &body)
            .map_err(|_| MicrosoftAuthError::InvalidResponse(MINECRAFT_LOGIN_OPERATION))?;
        if raw.access_token.trim().is_empty() {
            return Err(MicrosoftAuthError::InvalidResponse(
                MINECRAFT_LOGIN_OPERATION,
            ));
        }
        Ok(MinecraftToken {
            access_token: raw.access_token,
            expires_in: raw.expires_in,
        })
    }

    /// Reports whether the account owns Minecraft: Java Edition.
    pub async fn owns_minecraft(
        &self,
        minecraft_access_token: &str,
        cancellation: &CancellationToken,
    ) -> Result<bool, MicrosoftAuthError> {
        check_cancelled(cancellation)?;
        let request = self
            .client
            .get(&self.endpoints.minecraft_entitlements)
            .bearer_auth(minecraft_access_token);
        let (status, body) = self.execute(ENTITLEMENTS_OPERATION, request).await?;
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(MicrosoftAuthError::MinecraftTokenRejected);
        }
        if !status.is_success() {
            return Err(http_failure(ENTITLEMENTS_OPERATION, status, &body));
        }
        let raw: RawEntitlements = parse_json(ENTITLEMENTS_OPERATION, &body)
            .map_err(|_| MicrosoftAuthError::InvalidResponse(ENTITLEMENTS_OPERATION))?;
        Ok(raw
            .items
            .iter()
            .any(|item| item.name == "game_minecraft" || item.name == "product_minecraft"))
    }

    /// Fetches the Minecraft profile associated with an access token.
    pub async fn profile(
        &self,
        minecraft_access_token: &str,
        cancellation: &CancellationToken,
    ) -> Result<MicrosoftAccount, MicrosoftAuthError> {
        check_cancelled(cancellation)?;
        let request = self
            .client
            .get(&self.endpoints.minecraft_profile)
            .bearer_auth(minecraft_access_token);
        let (status, body) = self.execute(PROFILE_OPERATION, request).await?;
        if status == StatusCode::NOT_FOUND {
            return Err(MicrosoftAuthError::ProfileNotFound);
        }
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(MicrosoftAuthError::MinecraftTokenRejected);
        }
        if !status.is_success() {
            return Err(http_failure(PROFILE_OPERATION, status, &body));
        }
        parse_profile(PROFILE_OPERATION, &body)
    }

    /// Uploads a PNG skin and returns the updated profile.
    ///
    /// The caller is responsible for validating the image before calling this
    /// method. The official service expects a 64×64 or legacy 64×32 PNG.
    pub async fn upload_skin(
        &self,
        minecraft_access_token: &str,
        variant: SkinVariant,
        file_name: &str,
        bytes: &[u8],
        cancellation: &CancellationToken,
    ) -> Result<MicrosoftAccount, MicrosoftAuthError> {
        check_cancelled(cancellation)?;
        let file = reqwest::multipart::Part::bytes(bytes.to_vec())
            .file_name(file_name.to_owned())
            .mime_str("image/png")
            .map_err(|error| MicrosoftAuthError::InvalidSkinRequest {
                message: error.to_string(),
            })?;
        let form = reqwest::multipart::Form::new()
            .text("variant", variant.as_str())
            .part("file", file);
        let request = self
            .client
            .post(&self.endpoints.minecraft_profile_skins)
            .bearer_auth(minecraft_access_token)
            .multipart(form);
        self.execute_skin_mutation(SKIN_UPLOAD_OPERATION, request, cancellation)
            .await
    }

    /// Changes the active skin using an image URL.
    ///
    /// The URL must point at a PNG. The official service downloads it on its
    /// side, so callers that need guaranteed validation should download and
    /// upload the bytes instead.
    pub async fn change_skin_url(
        &self,
        minecraft_access_token: &str,
        variant: SkinVariant,
        url: &str,
        cancellation: &CancellationToken,
    ) -> Result<MicrosoftAccount, MicrosoftAuthError> {
        check_cancelled(cancellation)?;
        let request = self
            .client
            .post(&self.endpoints.minecraft_profile_skins)
            .bearer_auth(minecraft_access_token)
            .json(&ChangeSkinRequest {
                variant: variant.as_str(),
                url,
            });
        self.execute_skin_mutation(SKIN_CHANGE_OPERATION, request, cancellation)
            .await
    }

    /// Resets the active skin to the account default.
    ///
    /// Returns the updated profile when the service includes it. Callers
    /// should refresh the profile when this returns `None`.
    pub async fn reset_skin(
        &self,
        minecraft_access_token: &str,
        cancellation: &CancellationToken,
    ) -> Result<Option<MicrosoftAccount>, MicrosoftAuthError> {
        check_cancelled(cancellation)?;
        let request = self
            .client
            .delete(&self.endpoints.minecraft_profile_skins_active)
            .bearer_auth(minecraft_access_token);
        let (status, headers, body) = self
            .execute_with_headers(SKIN_RESET_OPERATION, request)
            .await?;
        if !status.is_success() {
            return Err(skin_failure(SKIN_RESET_OPERATION, status, &headers, &body));
        }
        if body.trim().is_empty() {
            return Ok(None);
        }
        parse_profile(SKIN_RESET_OPERATION, &body).map(Some)
    }

    async fn execute_skin_mutation(
        &self,
        operation: &'static str,
        request: RequestBuilder,
        cancellation: &CancellationToken,
    ) -> Result<MicrosoftAccount, MicrosoftAuthError> {
        check_cancelled(cancellation)?;
        let (status, headers, body) = self.execute_with_headers(operation, request).await?;
        if !status.is_success() {
            return Err(skin_failure(operation, status, &headers, &body));
        }
        if body.trim().is_empty() {
            return Err(MicrosoftAuthError::InvalidResponse(operation));
        }
        parse_profile(operation, &body)
    }

    /// Completes all post-OAuth exchanges and returns a launchable account.
    pub async fn complete_login(
        &self,
        oauth: MicrosoftOAuthTokens,
        cancellation: &CancellationToken,
    ) -> Result<AuthenticatedMicrosoft, MicrosoftAuthError> {
        let xbox = self
            .authenticate_xbox(&oauth.access_token, cancellation)
            .await?;
        let xsts = self.authorize_xsts(&xbox.token, cancellation).await?;
        let minecraft = self
            .login_minecraft(&xsts.user_hash, &xsts.token, cancellation)
            .await?;
        if !self
            .owns_minecraft(&minecraft.access_token, cancellation)
            .await?
        {
            return Err(MicrosoftAuthError::NoMinecraftEntitlement);
        }
        let account = self.profile(&minecraft.access_token, cancellation).await?;
        let credentials = MicrosoftCredentials::new(
            oauth.refresh_token,
            minecraft.access_token,
            unix_timestamp().saturating_add(minecraft.expires_in),
        );
        Ok(AuthenticatedMicrosoft {
            account,
            credentials,
        })
    }

    /// Refreshes an expired Minecraft token and re-resolves the profile.
    ///
    /// The returned credentials always contain the newest refresh token.
    pub async fn refresh_session(
        &self,
        credentials: &MicrosoftCredentials,
        cancellation: &CancellationToken,
    ) -> Result<AuthenticatedMicrosoft, MicrosoftAuthError> {
        let oauth = self.refresh_oauth(&credentials.refresh_token).await?;
        self.complete_login(oauth, cancellation).await
    }

    async fn execute(
        &self,
        operation: &'static str,
        request: RequestBuilder,
    ) -> Result<(StatusCode, String), MicrosoftAuthError> {
        let (status, _, body) = self.execute_with_headers(operation, request).await?;
        Ok((status, body))
    }

    async fn execute_with_headers(
        &self,
        operation: &'static str,
        request: RequestBuilder,
    ) -> Result<(StatusCode, reqwest::header::HeaderMap, String), MicrosoftAuthError> {
        let response = request
            .send()
            .await
            .map_err(|source| MicrosoftAuthError::Http { operation, source })?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .text()
            .await
            .map_err(|source| MicrosoftAuthError::Http { operation, source })?;
        Ok((status, headers, body))
    }
}

/// A pending device code login.
#[derive(Clone)]
pub struct DeviceLogin {
    auth: MicrosoftAuth,
    code: DeviceCode,
}

impl fmt::Debug for DeviceLogin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceLogin")
            .field("user_code", &self.code.user_code)
            .field("verification_uri", &self.code.verification_uri)
            .field("expires_in", &self.code.expires_in)
            .field("interval", &self.code.interval)
            .finish_non_exhaustive()
    }
}

impl DeviceLogin {
    fn new(auth: MicrosoftAuth, code: DeviceCode) -> Self {
        Self { auth, code }
    }

    pub fn user_code(&self) -> &str {
        &self.code.user_code
    }

    pub fn verification_uri(&self) -> &str {
        &self.code.verification_uri
    }

    pub fn expires_in(&self) -> Duration {
        Duration::from_secs(self.code.expires_in)
    }

    pub fn interval(&self) -> Duration {
        Duration::from_secs(self.code.interval)
    }

    /// Waits for the browser flow and completes the Minecraft exchanges.
    ///
    /// The wait honours the server-provided polling interval, increases it on
    /// `slow_down`, and stops at the device code expiry. Dropping the returned
    /// future cancels the pending HTTP request as well.
    pub async fn wait(
        self,
        cancellation: &CancellationToken,
    ) -> Result<AuthenticatedMicrosoft, MicrosoftAuthError> {
        let deadline = tokio::time::Instant::now() + self.expires_in();
        let mut interval = self.interval();
        let mut transient_failures = 0_u8;
        loop {
            check_cancelled(cancellation)?;
            if tokio::time::Instant::now() >= deadline {
                return Err(MicrosoftAuthError::DeviceCodeExpired);
            }
            sleep_or_cancel(interval, cancellation).await?;
            if tokio::time::Instant::now() >= deadline {
                return Err(MicrosoftAuthError::DeviceCodeExpired);
            }
            match self.auth.poll_device_code(&self.code.device_code).await {
                Ok(DeviceCodePoll::Pending) => {
                    transient_failures = 0;
                }
                Ok(DeviceCodePoll::SlowDown) => {
                    transient_failures = 0;
                    interval = interval.saturating_add(Duration::from_secs(5));
                }
                Ok(DeviceCodePoll::Complete(tokens)) => {
                    return self.auth.complete_login(tokens, cancellation).await;
                }
                Err(error) if error.is_transient() && transient_failures < 3 => {
                    transient_failures += 1;
                    interval = interval.saturating_add(Duration::from_secs(1));
                }
                Err(error) => return Err(error),
            }
        }
    }
}

/// One-time code returned by the OAuth device authorization endpoint.
#[derive(Clone)]
pub struct DeviceCode {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

impl fmt::Debug for DeviceCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceCode")
            .field("device_code", &"<redacted>")
            .field("user_code", &self.user_code)
            .field("verification_uri", &self.verification_uri)
            .field("expires_in", &self.expires_in)
            .field("interval", &self.interval)
            .finish()
    }
}

/// Result of one device code poll.
pub enum DeviceCodePoll {
    Pending,
    SlowDown,
    Complete(MicrosoftOAuthTokens),
}

/// Microsoft OAuth tokens returned by the device code or refresh grant.
#[derive(Clone, PartialEq, Eq)]
pub struct MicrosoftOAuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
}

impl fmt::Debug for MicrosoftOAuthTokens {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MicrosoftOAuthTokens")
            .field("access_token", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

/// Xbox Live or XSTS token together with the user hash.
#[derive(Clone, PartialEq, Eq)]
pub struct XboxToken {
    pub token: String,
    pub user_hash: String,
}

impl fmt::Debug for XboxToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("XboxToken")
            .field("token", &"<redacted>")
            .field("user_hash", &self.user_hash)
            .finish()
    }
}

/// Minecraft services access token.
#[derive(Clone, PartialEq, Eq)]
pub struct MinecraftToken {
    pub access_token: String,
    pub expires_in: u64,
}

impl fmt::Debug for MinecraftToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MinecraftToken")
            .field("access_token", &"<redacted>")
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

/// The arm model used by an official Java skin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkinVariant {
    Classic,
    Slim,
}

impl SkinVariant {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Slim => "slim",
        }
    }

    fn from_service(value: &str) -> Self {
        if value.eq_ignore_ascii_case("SLIM") {
            Self::Slim
        } else {
            Self::Classic
        }
    }
}

/// One skin owned by an official account.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProfileSkin {
    id: String,
    url: String,
    variant: SkinVariant,
    alias: Option<String>,
    active: bool,
}

impl ProfileSkin {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn variant(&self) -> SkinVariant {
        self.variant
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    pub fn is_active(&self) -> bool {
        self.active
    }
}

/// One cape owned by an official account.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProfileCape {
    id: String,
    url: String,
    alias: Option<String>,
    active: bool,
}

impl ProfileCape {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    pub fn is_active(&self) -> bool {
        self.active
    }
}

/// A verified Minecraft profile owned by a Microsoft account.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MicrosoftAccount {
    username: String,
    uuid: Uuid,
    skin_url: Option<String>,
    skins: Vec<ProfileSkin>,
    capes: Vec<ProfileCape>,
}

impl MicrosoftAccount {
    pub fn new(username: impl Into<String>, uuid: Uuid) -> Result<Self, MicrosoftAccountError> {
        let username = username.into();
        if username.trim().is_empty() {
            return Err(MicrosoftAccountError::BlankUsername);
        }
        Ok(Self {
            username,
            uuid,
            skin_url: None,
            skins: Vec::new(),
            capes: Vec::new(),
        })
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    pub fn compact_uuid(&self) -> String {
        self.uuid.simple().to_string()
    }

    /// Official skin texture URL from the Minecraft profile, when one is active.
    pub fn skin_url(&self) -> Option<&str> {
        self.skin_url.as_deref()
    }

    pub fn with_skin_url(mut self, skin_url: Option<String>) -> Self {
        self.skin_url = skin_url;
        self
    }

    /// All skins owned by this account, including inactive entries.
    pub fn skins(&self) -> &[ProfileSkin] {
        &self.skins
    }

    /// All capes owned by this account, including inactive entries.
    pub fn capes(&self) -> &[ProfileCape] {
        &self.capes
    }

    pub fn active_skin(&self) -> Option<&ProfileSkin> {
        self.skins.iter().find(|skin| skin.is_active())
    }

    /// Replaces the owned skin list and derives the active skin URL from it.
    pub fn with_profile_skins(mut self, skins: Vec<ProfileSkin>) -> Self {
        self.skin_url = skins
            .iter()
            .find(|skin| skin.is_active())
            .map(|skin| skin.url.clone());
        self.skins = skins;
        self
    }

    pub fn with_capes(mut self, capes: Vec<ProfileCape>) -> Self {
        self.capes = capes;
        self
    }

    pub fn with_username(
        &self,
        username: impl Into<String>,
    ) -> Result<Self, MicrosoftAccountError> {
        let username = username.into();
        if username.trim().is_empty() {
            return Err(MicrosoftAccountError::BlankUsername);
        }
        Ok(Self {
            username,
            ..self.clone()
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MicrosoftAccountError {
    #[error("Minecraft profile name cannot be blank")]
    BlankUsername,
}

/// Long-lived credentials for one Microsoft account.
///
/// This value is serialized into the operating system credential store. It
/// must never be written to the launcher configuration file.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MicrosoftCredentials {
    pub version: u32,
    pub refresh_token: String,
    pub minecraft_access_token: String,
    pub minecraft_expires_at: u64,
}

impl fmt::Debug for MicrosoftCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MicrosoftCredentials")
            .field("version", &self.version)
            .field("refresh_token", &"<redacted>")
            .field("minecraft_access_token", &"<redacted>")
            .field("minecraft_expires_at", &self.minecraft_expires_at)
            .finish()
    }
}

impl MicrosoftCredentials {
    pub const SCHEMA_VERSION: u32 = 1;

    pub fn new(
        refresh_token: impl Into<String>,
        minecraft_access_token: impl Into<String>,
        minecraft_expires_at: u64,
    ) -> Self {
        Self {
            version: Self::SCHEMA_VERSION,
            refresh_token: refresh_token.into(),
            minecraft_access_token: minecraft_access_token.into(),
            minecraft_expires_at,
        }
    }

    /// Returns whether the cached Minecraft token is still safe to use.
    pub fn minecraft_token_valid(&self) -> bool {
        self.minecraft_token_valid_at(unix_timestamp())
    }

    pub fn minecraft_token_valid_at(&self, now: u64) -> bool {
        self.minecraft_expires_at > now.saturating_add(MINECRAFT_TOKEN_REFRESH_MARGIN_SECS)
    }
}

/// Result of a completed Microsoft login or refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedMicrosoft {
    pub account: MicrosoftAccount,
    pub credentials: MicrosoftCredentials,
}

#[derive(Debug, Error)]
pub enum MicrosoftAuthError {
    #[error("Microsoft OAuth client ID cannot be blank")]
    InvalidClientId,
    #[error("failed to create the Microsoft HTTP client: {0}")]
    Client(#[source] reqwest::Error),
    #[error("{operation} failed: {source}")]
    Http {
        operation: &'static str,
        #[source]
        source: reqwest::Error,
    },
    #[error("{operation} returned HTTP {status}: {body}")]
    HttpStatus {
        operation: &'static str,
        status: StatusCode,
        body: String,
    },
    #[error("Microsoft OAuth error `{code}`: {message}")]
    OAuth { code: String, message: String },
    #[error("{0} returned an invalid response")]
    InvalidResponse(&'static str),
    #[error("Microsoft login expired; start the login again")]
    DeviceCodeExpired,
    #[error("Microsoft login was declined")]
    DeviceCodeDeclined,
    #[error("the Microsoft session is no longer valid; sign in again")]
    ReauthenticationRequired,
    #[error("Xbox Live rejected the account (XErr {code:?}): {message}")]
    Xbox {
        code: Option<i64>,
        message: String,
        redirect: Option<String>,
    },
    #[error("this Microsoft account does not own Minecraft: Java Edition")]
    NoMinecraftEntitlement,
    #[error(
        "this Microsoft account has no Minecraft profile; sign in to the official launcher once to create one"
    )]
    ProfileNotFound,
    #[error("Minecraft services rejected the access token")]
    MinecraftTokenRejected,
    #[error("Minecraft services rejected the skin: {message}")]
    SkinRejected { message: String },
    #[error("Minecraft services rate-limited skin changes")]
    SkinRateLimited { retry_after: Option<u64> },
    #[error("Minecraft services refused the skin change: {message}")]
    SkinForbidden { message: String },
    #[error("invalid skin upload request: {message}")]
    InvalidSkinRequest { message: String },
    #[error("Microsoft login was cancelled")]
    Cancelled,
}

impl MicrosoftAuthError {
    /// Whether retrying the same request may succeed without user action.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Http { .. } => true,
            Self::HttpStatus { status, .. } => status.is_server_error(),
            _ => false,
        }
    }
}

fn parse_json<T: for<'de> Deserialize<'de>>(
    operation: &'static str,
    body: &str,
) -> Result<T, MicrosoftAuthError> {
    serde_json::from_str(body).map_err(|_| MicrosoftAuthError::InvalidResponse(operation))
}

fn parse_profile(
    operation: &'static str,
    body: &str,
) -> Result<MicrosoftAccount, MicrosoftAuthError> {
    let raw: RawProfile = parse_json(operation, body)?;
    let uuid =
        Uuid::parse_str(&raw.id).map_err(|_| MicrosoftAuthError::InvalidResponse(operation))?;
    let skins = raw
        .skins
        .into_iter()
        .filter(|skin| !skin.url.trim().is_empty())
        .map(|skin| ProfileSkin {
            id: skin.id,
            url: skin.url,
            variant: SkinVariant::from_service(&skin.variant),
            alias: skin.alias.filter(|alias| !alias.trim().is_empty()),
            active: skin.state.eq_ignore_ascii_case("ACTIVE"),
        })
        .collect();
    let capes = raw
        .capes
        .into_iter()
        .filter(|cape| !cape.url.trim().is_empty())
        .map(|cape| ProfileCape {
            id: cape.id,
            url: cape.url,
            alias: cape.alias.filter(|alias| !alias.trim().is_empty()),
            active: cape.state.eq_ignore_ascii_case("ACTIVE"),
        })
        .collect();
    MicrosoftAccount::new(raw.name, uuid)
        .map(|account| account.with_profile_skins(skins).with_capes(capes))
        .map_err(|_| MicrosoftAuthError::InvalidResponse(operation))
}

fn skin_failure(
    operation: &'static str,
    status: StatusCode,
    headers: &reqwest::header::HeaderMap,
    body: &str,
) -> MicrosoftAuthError {
    let message = api_error_message(body);
    if status == StatusCode::UNAUTHORIZED {
        return MicrosoftAuthError::MinecraftTokenRejected;
    }
    if status == StatusCode::FORBIDDEN {
        if is_account_suspended(body) {
            return MicrosoftAuthError::SkinForbidden { message };
        }
        return MicrosoftAuthError::MinecraftTokenRejected;
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return MicrosoftAuthError::SkinRateLimited {
            retry_after: retry_after(headers),
        };
    }
    if status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY {
        return MicrosoftAuthError::SkinRejected { message };
    }
    if status == StatusCode::NOT_FOUND {
        return MicrosoftAuthError::ProfileNotFound;
    }
    MicrosoftAuthError::HttpStatus {
        operation,
        status,
        body: body.to_owned(),
    }
}

fn api_error_message(body: &str) -> String {
    let parsed: Option<RawApiError> = serde_json::from_str(body).ok();
    let message = parsed
        .and_then(|error| {
            if !error.error_message.trim().is_empty() {
                Some(error.error_message)
            } else if !error.error.trim().is_empty() {
                Some(error.error)
            } else {
                None
            }
        })
        .unwrap_or_else(|| body.trim().to_owned());
    truncate_message(message, 512)
}

fn truncate_message(mut message: String, limit: usize) -> String {
    if message.len() > limit {
        let mut boundary = limit;
        while !message.is_char_boundary(boundary) {
            boundary -= 1;
        }
        message.truncate(boundary);
        message.push('…');
    }
    message
}

fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse().ok())
}

fn is_account_suspended(body: &str) -> bool {
    body.contains("ACCOUNT_SUSPENDED")
        || serde_json::from_str::<RawApiError>(body)
            .ok()
            .is_some_and(|error| {
                error
                    .details
                    .reason
                    .eq_ignore_ascii_case("ACCOUNT_SUSPENDED")
            })
}

fn parse_oauth_tokens(
    operation: &'static str,
    body: &str,
    previous_refresh_token: Option<&str>,
) -> Result<MicrosoftOAuthTokens, MicrosoftAuthError> {
    let raw: RawOAuthTokens = parse_json(operation, body)?;
    if raw.access_token.trim().is_empty() {
        return Err(MicrosoftAuthError::InvalidResponse(operation));
    }
    let refresh_token = raw
        .refresh_token
        .filter(|token| !token.trim().is_empty())
        .or_else(|| previous_refresh_token.map(str::to_owned))
        .ok_or(MicrosoftAuthError::InvalidResponse(operation))?;
    Ok(MicrosoftOAuthTokens {
        access_token: raw.access_token,
        refresh_token,
        expires_in: raw.expires_in,
    })
}

fn parse_xbox_token(operation: &'static str, body: &str) -> Result<XboxToken, MicrosoftAuthError> {
    let raw: RawXboxToken = parse_json(operation, body)?;
    let user_hash = raw
        .display_claims
        .xui
        .first()
        .map(|claim| claim.uhs.clone())
        .filter(|uhs| !uhs.trim().is_empty())
        .ok_or(MicrosoftAuthError::InvalidResponse(operation))?;
    if raw.token.trim().is_empty() {
        return Err(MicrosoftAuthError::InvalidResponse(operation));
    }
    Ok(XboxToken {
        token: raw.token,
        user_hash,
    })
}

fn oauth_failure(operation: &'static str, status: StatusCode, body: &str) -> MicrosoftAuthError {
    match serde_json::from_str::<RawOAuthError>(body) {
        Ok(error) => MicrosoftAuthError::OAuth {
            code: error.error,
            message: error.error_description.unwrap_or_default(),
        },
        Err(_) => http_failure(operation, status, body),
    }
}

fn http_failure(operation: &'static str, status: StatusCode, body: &str) -> MicrosoftAuthError {
    MicrosoftAuthError::HttpStatus {
        operation,
        status,
        body: truncated(body),
    }
}

fn truncated(body: &str) -> String {
    const LIMIT: usize = 512;
    let mut value = body.chars().take(LIMIT).collect::<String>();
    if body.chars().count() > LIMIT {
        value.push('…');
    }
    value
}

fn check_cancelled(cancellation: &CancellationToken) -> Result<(), MicrosoftAuthError> {
    if cancellation.is_cancelled() {
        Err(MicrosoftAuthError::Cancelled)
    } else {
        Ok(())
    }
}

async fn sleep_or_cancel(
    duration: Duration,
    cancellation: &CancellationToken,
) -> Result<(), MicrosoftAuthError> {
    tokio::select! {
        _ = cancellation.cancelled() => Err(MicrosoftAuthError::Cancelled),
        _ = tokio::time::sleep(duration) => Ok(()),
    }
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Deserialize)]
struct RawDeviceCode {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: Option<u64>,
}

#[derive(Deserialize)]
struct RawOAuthError {
    error: String,
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct RawOAuthTokens {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
}

#[derive(Serialize)]
struct XboxUserAuthRequest<'a> {
    #[serde(rename = "Properties")]
    properties: XboxUserAuthProperties<'a>,
    #[serde(rename = "RelyingParty")]
    relying_party: &'a str,
    #[serde(rename = "TokenType")]
    token_type: &'a str,
}

#[derive(Serialize)]
struct XboxUserAuthProperties<'a> {
    #[serde(rename = "AuthMethod")]
    auth_method: &'a str,
    #[serde(rename = "SiteName")]
    site_name: &'a str,
    #[serde(rename = "RpsTicket")]
    rps_ticket: String,
}

#[derive(Serialize)]
struct XstsRequest<'a> {
    #[serde(rename = "Properties")]
    properties: XstsProperties<'a>,
    #[serde(rename = "RelyingParty")]
    relying_party: &'a str,
    #[serde(rename = "TokenType")]
    token_type: &'a str,
}

#[derive(Serialize)]
struct XstsProperties<'a> {
    #[serde(rename = "SandboxId")]
    sandbox_id: &'a str,
    #[serde(rename = "UserTokens")]
    user_tokens: [&'a str; 1],
}

#[derive(Serialize)]
struct MinecraftLoginRequest {
    #[serde(rename = "identityToken")]
    identity_token: String,
}

#[derive(Deserialize)]
struct RawXboxToken {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: RawDisplayClaims,
}

#[derive(Deserialize)]
struct RawDisplayClaims {
    #[serde(default)]
    xui: Vec<RawXuiClaim>,
}

#[derive(Deserialize)]
struct RawXuiClaim {
    #[serde(default)]
    uhs: String,
}

#[derive(Deserialize)]
struct RawXstsError {
    #[serde(rename = "XErr")]
    xerr: Option<i64>,
    #[serde(rename = "Message")]
    message: Option<String>,
    #[serde(rename = "Redirect")]
    redirect: Option<String>,
}

#[derive(Deserialize)]
struct RawMinecraftToken {
    access_token: String,
    expires_in: u64,
}

#[derive(Deserialize)]
struct RawEntitlements {
    #[serde(default)]
    items: Vec<RawEntitlement>,
}

#[derive(Deserialize)]
struct RawEntitlement {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct RawProfile {
    id: String,
    name: String,
    #[serde(default)]
    skins: Vec<RawProfileSkin>,
    #[serde(default)]
    capes: Vec<RawProfileCape>,
}

#[derive(Deserialize)]
struct RawProfileSkin {
    #[serde(default)]
    id: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    variant: String,
    #[serde(default)]
    alias: Option<String>,
}

#[derive(Deserialize)]
struct RawProfileCape {
    #[serde(default)]
    id: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    alias: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawApiError {
    #[serde(default)]
    error: String,
    #[serde(default, rename = "errorMessage")]
    error_message: String,
    #[serde(default)]
    details: RawApiErrorDetails,
}

#[derive(Deserialize, Default)]
struct RawApiErrorDetails {
    #[serde(default)]
    reason: String,
}

#[derive(Serialize)]
struct ChangeSkinRequest<'a> {
    variant: &'a str,
    url: &'a str,
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, VecDeque},
        sync::{Arc, Mutex},
    };

    use axum::{
        Router,
        body::Bytes,
        extract::State,
        http::{
            HeaderMap, StatusCode,
            header::{AUTHORIZATION, CONTENT_TYPE},
        },
        response::{IntoResponse, Response},
        routing::{delete, get, post},
    };
    use serde_json::{Value, json};

    use super::*;

    const PROFILE_ID: &str = "986dec87b7ec47ff89ff033fdb95c4b5";
    const PROFILE_NAME: &str = "HowDoesAuthWork";

    #[derive(Clone)]
    struct MockState {
        requests: Arc<Mutex<Vec<RecordedRequest>>>,
        token_replies: Arc<Mutex<VecDeque<MockReply>>>,
        entitlements: Arc<Mutex<Value>>,
        profile: Arc<Mutex<MockReply>>,
        skin_change: Arc<Mutex<MockReply>>,
        skin_reset: Arc<Mutex<MockReply>>,
        xsts: Arc<Mutex<MockReply>>,
    }

    #[derive(Clone)]
    struct MockReply {
        status: StatusCode,
        body: Value,
        headers: Vec<(&'static str, String)>,
    }

    impl MockReply {
        fn ok(body: Value) -> Self {
            Self {
                status: StatusCode::OK,
                body,
                headers: Vec::new(),
            }
        }

        fn status(status: StatusCode, body: Value) -> Self {
            Self {
                status,
                body,
                headers: Vec::new(),
            }
        }

        fn with_header(mut self, name: &'static str, value: impl Into<String>) -> Self {
            self.headers.push((name, value.into()));
            self
        }
    }

    #[derive(Clone, Debug)]
    struct RecordedRequest {
        path: String,
        body: String,
        authorization: Option<String>,
        content_type: Option<String>,
    }

    impl MockState {
        fn new() -> Self {
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                token_replies: Arc::new(Mutex::new(VecDeque::new())),
                entitlements: Arc::new(Mutex::new(json!({
                    "items": [
                        {"name": "product_minecraft"},
                        {"name": "game_minecraft"}
                    ]
                }))),
                profile: Arc::new(Mutex::new(MockReply::ok(json!({
                    "id": PROFILE_ID,
                    "name": PROFILE_NAME,
                    "skins": [{
                        "state": "ACTIVE",
                        "url": "http://textures.minecraft.net/texture/abc",
                        "variant": "CLASSIC"
                    }]
                })))),
                skin_change: Arc::new(Mutex::new(MockReply::ok(json!({
                    "id": PROFILE_ID,
                    "name": PROFILE_NAME,
                    "skins": [{
                        "state": "ACTIVE",
                        "url": "http://textures.minecraft.net/texture/new",
                        "variant": "SLIM"
                    }]
                })))),
                skin_reset: Arc::new(Mutex::new(MockReply::ok(json!({
                    "id": PROFILE_ID,
                    "name": PROFILE_NAME,
                    "skins": [{
                        "state": "ACTIVE",
                        "url": "http://textures.minecraft.net/texture/default",
                        "variant": "CLASSIC"
                    }]
                })))),
                xsts: Arc::new(Mutex::new(MockReply::ok(xsts_success()))),
            }
        }

        fn push_token_reply(&self, reply: MockReply) {
            self.token_replies.lock().unwrap().push_back(reply);
        }

        fn requests(&self) -> Vec<RecordedRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    fn xsts_success() -> Value {
        json!({
            "Token": "xsts-token",
            "DisplayClaims": {"xui": [{"uhs": "uhs-123"}]}
        })
    }

    fn oauth_success(refresh_token: Option<&str>) -> Value {
        let mut body = json!({
            "access_token": "msa-access",
            "expires_in": 3600,
            "token_type": "Bearer"
        });
        if let Some(refresh_token) = refresh_token {
            body["refresh_token"] = json!(refresh_token);
        }
        body
    }

    async fn spawn_mock(state: MockState) -> String {
        let router = Router::new()
            .route("/oauth/devicecode", post(mock_device_code))
            .route("/oauth/token", post(mock_token))
            .route("/xbox/authenticate", post(mock_xbox))
            .route("/xsts/authorize", post(mock_xsts))
            .route("/minecraft/login", post(mock_minecraft_login))
            .route("/minecraft/entitlements", get(mock_entitlements))
            .route("/minecraft/profile", get(mock_profile))
            .route("/minecraft/profile/skins", post(mock_skin_change))
            .route("/minecraft/profile/skins/active", delete(mock_skin_reset))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        format!("http://{address}")
    }

    fn endpoints(base: &str) -> MicrosoftEndpoints {
        MicrosoftEndpoints {
            device_code: format!("{base}/oauth/devicecode"),
            token: format!("{base}/oauth/token"),
            xbox_authenticate: format!("{base}/xbox/authenticate"),
            xsts_authorize: format!("{base}/xsts/authorize"),
            minecraft_login: format!("{base}/minecraft/login"),
            minecraft_entitlements: format!("{base}/minecraft/entitlements"),
            minecraft_profile: format!("{base}/minecraft/profile"),
            minecraft_profile_skins: format!("{base}/minecraft/profile/skins"),
            minecraft_profile_skins_active: format!("{base}/minecraft/profile/skins/active"),
        }
    }

    fn auth(base: &str) -> MicrosoftAuth {
        MicrosoftAuth::with_endpoints("test-client", endpoints(base)).unwrap()
    }

    fn device_login(auth: MicrosoftAuth) -> DeviceLogin {
        DeviceLogin::new(
            auth,
            DeviceCode {
                device_code: "device-secret".to_owned(),
                user_code: "ABCD-EFGH".to_owned(),
                verification_uri: "https://www.microsoft.com/link".to_owned(),
                expires_in: 900,
                interval: 0,
            },
        )
    }

    async fn record(
        state: &MockState,
        path: &str,
        headers: &HeaderMap,
        body: &Bytes,
    ) -> RecordedRequest {
        let request = RecordedRequest {
            path: path.to_owned(),
            body: String::from_utf8_lossy(body).into_owned(),
            authorization: headers
                .get(AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
            content_type: headers
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        };
        state.requests.lock().unwrap().push(request.clone());
        request
    }

    fn reply(reply: MockReply) -> Response {
        let mut response = (reply.status, axum::Json(reply.body)).into_response();
        for (name, value) in reply.headers {
            if let (Ok(name), Ok(value)) = (
                axum::http::HeaderName::from_bytes(name.as_bytes()),
                axum::http::HeaderValue::from_str(&value),
            ) {
                response.headers_mut().insert(name, value);
            }
        }
        response
    }

    async fn mock_device_code(
        State(state): State<MockState>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        record(&state, "/oauth/devicecode", &headers, &body).await;
        reply(MockReply::ok(json!({
            "device_code": "device-secret",
            "user_code": "ABCD-EFGH",
            "verification_uri": "https://www.microsoft.com/link",
            "expires_in": 900,
            "interval": 5
        })))
    }

    async fn mock_token(
        State(state): State<MockState>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        record(&state, "/oauth/token", &headers, &body).await;
        let response = state
            .token_replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| MockReply::ok(oauth_success(Some("msa-refresh"))));
        reply(response)
    }

    async fn mock_xbox(
        State(state): State<MockState>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        record(&state, "/xbox/authenticate", &headers, &body).await;
        reply(MockReply::ok(json!({
            "Token": "xbl-token",
            "DisplayClaims": {"xui": [{"uhs": "uhs-123"}]}
        })))
    }

    async fn mock_xsts(
        State(state): State<MockState>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        record(&state, "/xsts/authorize", &headers, &body).await;
        reply(state.xsts.lock().unwrap().clone())
    }

    async fn mock_minecraft_login(
        State(state): State<MockState>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        record(&state, "/minecraft/login", &headers, &body).await;
        reply(MockReply::ok(json!({
            "username": "ignored",
            "access_token": "minecraft-access",
            "token_type": "Bearer",
            "expires_in": 86400
        })))
    }

    async fn mock_entitlements(State(state): State<MockState>, headers: HeaderMap) -> Response {
        record(&state, "/minecraft/entitlements", &headers, &Bytes::new()).await;
        reply(MockReply::ok(state.entitlements.lock().unwrap().clone()))
    }

    async fn mock_profile(State(state): State<MockState>, headers: HeaderMap) -> Response {
        record(&state, "/minecraft/profile", &headers, &Bytes::new()).await;
        reply(state.profile.lock().unwrap().clone())
    }

    async fn mock_skin_change(
        State(state): State<MockState>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        record(&state, "/minecraft/profile/skins", &headers, &body).await;
        reply(state.skin_change.lock().unwrap().clone())
    }

    async fn mock_skin_reset(State(state): State<MockState>, headers: HeaderMap) -> Response {
        record(
            &state,
            "/minecraft/profile/skins/active",
            &headers,
            &Bytes::new(),
        )
        .await;
        reply(state.skin_reset.lock().unwrap().clone())
    }

    fn form(request: &RecordedRequest) -> HashMap<String, String> {
        url::form_urlencoded::parse(request.body.as_bytes())
            .into_owned()
            .collect()
    }

    fn json_body(request: &RecordedRequest) -> Value {
        serde_json::from_str(&request.body).unwrap()
    }

    #[tokio::test]
    async fn device_code_request_uses_live_flow_contract() {
        let state = MockState::new();
        let base = spawn_mock(state.clone()).await;
        let auth = auth(&base);

        let code = auth.request_device_code().await.unwrap();

        assert_eq!(code.user_code, "ABCD-EFGH");
        assert_eq!(code.verification_uri, "https://www.microsoft.com/link");
        assert_eq!(code.expires_in, 900);
        assert_eq!(code.interval, 5);
        assert!(!format!("{code:?}").contains("device-secret"));

        let requests = state.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/oauth/devicecode");
        let form = form(&requests[0]);
        assert_eq!(form["client_id"], "test-client");
        assert_eq!(form["scope"], MICROSOFT_SCOPE);
        assert_eq!(form["response_type"], "device_code");
    }

    #[tokio::test]
    async fn device_login_polls_until_tokens_are_issued() {
        let state = MockState::new();
        state.push_token_reply(MockReply::status(
            StatusCode::BAD_REQUEST,
            json!({"error": "authorization_pending"}),
        ));
        state.push_token_reply(MockReply::ok(oauth_success(Some("msa-refresh"))));
        let base = spawn_mock(state.clone()).await;
        let login = device_login(auth(&base));

        let authenticated = login.wait(&CancellationToken::new()).await.unwrap();

        assert_eq!(authenticated.account.username(), PROFILE_NAME);
        let requests = state.requests();
        assert_eq!(requests.len(), 7);
        let polls = requests
            .iter()
            .filter(|request| request.path == "/oauth/token")
            .count();
        assert_eq!(polls, 2);
        let poll = form(&requests[0]);
        assert_eq!(
            poll["grant_type"],
            "urn:ietf:params:oauth:grant-type:device_code"
        );
        assert_eq!(poll["device_code"], "device-secret");
    }

    #[tokio::test]
    async fn cancelled_device_login_stops_without_polling() {
        let state = MockState::new();
        let base = spawn_mock(state.clone()).await;
        let login = device_login(auth(&base));
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        assert!(matches!(
            login.wait(&cancellation).await,
            Err(MicrosoftAuthError::Cancelled)
        ));
        assert!(state.requests().is_empty());
    }

    #[tokio::test]
    async fn complete_login_uses_documented_exchange_contract() {
        let state = MockState::new();
        let base = spawn_mock(state.clone()).await;
        let auth = auth(&base);
        let oauth = MicrosoftOAuthTokens {
            access_token: "msa-access".to_owned(),
            refresh_token: "msa-refresh".to_owned(),
            expires_in: 3600,
        };
        let before = unix_timestamp();

        let authenticated = auth
            .complete_login(oauth, &CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(authenticated.account.username(), PROFILE_NAME);
        assert_eq!(
            authenticated.account.uuid(),
            Uuid::parse_str(PROFILE_ID).unwrap()
        );
        assert_eq!(
            authenticated.account.skin_url(),
            Some("http://textures.minecraft.net/texture/abc")
        );
        assert_eq!(authenticated.credentials.refresh_token, "msa-refresh");
        assert_eq!(
            authenticated.credentials.minecraft_access_token,
            "minecraft-access"
        );
        assert!(authenticated.credentials.minecraft_expires_at >= before + 86400);
        assert!(authenticated.credentials.minecraft_expires_at <= unix_timestamp() + 86400);

        let requests = state.requests();
        assert_eq!(requests.len(), 5);

        assert_eq!(requests[0].path, "/xbox/authenticate");
        let xbox = json_body(&requests[0]);
        assert_eq!(xbox["Properties"]["AuthMethod"], "RPS");
        assert_eq!(xbox["Properties"]["SiteName"], "user.auth.xboxlive.com");
        assert_eq!(xbox["Properties"]["RpsTicket"], "d=msa-access");
        assert_eq!(xbox["RelyingParty"], "http://auth.xboxlive.com");
        assert_eq!(xbox["TokenType"], "JWT");

        assert_eq!(requests[1].path, "/xsts/authorize");
        let xsts = json_body(&requests[1]);
        assert_eq!(xsts["Properties"]["SandboxId"], "RETAIL");
        assert_eq!(xsts["Properties"]["UserTokens"][0], "xbl-token");
        assert_eq!(xsts["RelyingParty"], "rp://api.minecraftservices.com/");

        assert_eq!(requests[2].path, "/minecraft/login");
        assert_eq!(
            json_body(&requests[2])["identityToken"],
            "XBL3.0 x=uhs-123;xsts-token"
        );

        assert_eq!(requests[3].path, "/minecraft/entitlements");
        assert_eq!(
            requests[3].authorization.as_deref(),
            Some("Bearer minecraft-access")
        );

        assert_eq!(requests[4].path, "/minecraft/profile");
        assert_eq!(
            requests[4].authorization.as_deref(),
            Some("Bearer minecraft-access")
        );
    }

    #[tokio::test]
    async fn refresh_keeps_the_previous_refresh_token_when_not_rotated() {
        let state = MockState::new();
        state.push_token_reply(MockReply::ok(oauth_success(None)));
        let base = spawn_mock(state.clone()).await;
        let auth = auth(&base);

        let tokens = auth.refresh_oauth("old-refresh").await.unwrap();

        assert_eq!(tokens.access_token, "msa-access");
        assert_eq!(tokens.refresh_token, "old-refresh");
        assert_eq!(tokens.expires_in, 3600);
        let requests = state.requests();
        let form = form(&requests[0]);
        assert_eq!(form["grant_type"], "refresh_token");
        assert_eq!(form["refresh_token"], "old-refresh");
        assert_eq!(form["scope"], MICROSOFT_SCOPE);
    }

    #[tokio::test]
    async fn device_code_errors_are_typed() {
        let state = MockState::new();
        state.push_token_reply(MockReply::status(
            StatusCode::BAD_REQUEST,
            json!({"error": "slow_down"}),
        ));
        state.push_token_reply(MockReply::status(
            StatusCode::BAD_REQUEST,
            json!({"error": "expired_token"}),
        ));
        state.push_token_reply(MockReply::status(
            StatusCode::BAD_REQUEST,
            json!({"error": "authorization_declined"}),
        ));
        let base = spawn_mock(state).await;
        let auth = auth(&base);

        assert!(matches!(
            auth.poll_device_code("device-secret").await.unwrap(),
            DeviceCodePoll::SlowDown
        ));
        assert!(matches!(
            auth.poll_device_code("device-secret").await,
            Err(MicrosoftAuthError::DeviceCodeExpired)
        ));
        assert!(matches!(
            auth.poll_device_code("device-secret").await,
            Err(MicrosoftAuthError::DeviceCodeDeclined)
        ));
    }

    #[tokio::test]
    async fn invalid_refresh_token_requires_reauthentication() {
        let state = MockState::new();
        state.push_token_reply(MockReply::status(
            StatusCode::BAD_REQUEST,
            json!({"error": "invalid_grant", "error_description": "expired"}),
        ));
        let base = spawn_mock(state).await;
        let auth = auth(&base);

        assert!(matches!(
            auth.refresh_oauth("old-refresh").await,
            Err(MicrosoftAuthError::ReauthenticationRequired)
        ));
    }

    #[tokio::test]
    async fn account_without_java_entitlement_is_rejected() {
        let state = MockState::new();
        *state.entitlements.lock().unwrap() = json!({
            "items": [{"name": "game_minecraft_bedrock"}]
        });
        let base = spawn_mock(state).await;
        let auth = auth(&base);
        let oauth = MicrosoftOAuthTokens {
            access_token: "msa-access".to_owned(),
            refresh_token: "msa-refresh".to_owned(),
            expires_in: 3600,
        };

        assert!(matches!(
            auth.complete_login(oauth, &CancellationToken::new()).await,
            Err(MicrosoftAuthError::NoMinecraftEntitlement)
        ));
    }

    #[tokio::test]
    async fn xsts_family_error_exposes_the_xbox_code() {
        let state = MockState::new();
        *state.xsts.lock().unwrap() = MockReply::status(
            StatusCode::UNAUTHORIZED,
            json!({
                "Identity": "0",
                "XErr": 2148916238_i64,
                "Message": "",
                "Redirect": "https://start.ui.xboxlive.com/AddChildToFamily"
            }),
        );
        let base = spawn_mock(state).await;
        let auth = auth(&base);
        let oauth = MicrosoftOAuthTokens {
            access_token: "msa-access".to_owned(),
            refresh_token: "msa-refresh".to_owned(),
            expires_in: 3600,
        };

        let error = auth
            .complete_login(oauth, &CancellationToken::new())
            .await
            .unwrap_err();
        match error {
            MicrosoftAuthError::Xbox {
                code,
                redirect,
                message,
            } => {
                assert_eq!(code, Some(2148916238));
                assert_eq!(message, "");
                assert_eq!(
                    redirect.as_deref(),
                    Some("https://start.ui.xboxlive.com/AddChildToFamily")
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn inactive_skin_entries_do_not_become_the_active_skin() {
        let state = MockState::new();
        *state.profile.lock().unwrap() = MockReply::ok(json!({
            "id": PROFILE_ID,
            "name": PROFILE_NAME,
            "skins": [
                {"state": "INACTIVE", "url": "http://textures.minecraft.net/texture/old"},
                {"state": "ACTIVE", "url": ""}
            ]
        }));
        let base = spawn_mock(state).await;
        let auth = auth(&base);
        let oauth = MicrosoftOAuthTokens {
            access_token: "msa-access".to_owned(),
            refresh_token: "msa-refresh".to_owned(),
            expires_in: 3600,
        };

        let authenticated = auth
            .complete_login(oauth, &CancellationToken::new())
            .await
            .unwrap();

        assert!(authenticated.account.skin_url().is_none());
        assert_eq!(authenticated.account.skins().len(), 1);
        assert!(!authenticated.account.skins()[0].is_active());
    }

    #[tokio::test]
    async fn profile_exposes_owned_skins_and_capes() {
        let state = MockState::new();
        *state.profile.lock().unwrap() = MockReply::ok(json!({
            "id": PROFILE_ID,
            "name": PROFILE_NAME,
            "skins": [
                {
                    "id": "skin-active",
                    "state": "ACTIVE",
                    "url": "http://textures.minecraft.net/texture/new",
                    "variant": "SLIM",
                    "alias": "My Skin"
                },
                {
                    "id": "skin-old",
                    "state": "INACTIVE",
                    "url": "http://textures.minecraft.net/texture/old",
                    "variant": "CLASSIC"
                }
            ],
            "capes": [
                {
                    "id": "cape-active",
                    "state": "ACTIVE",
                    "url": "http://textures.minecraft.net/texture/cape",
                    "alias": "Migrator"
                },
                {
                    "id": "cape-old",
                    "state": "INACTIVE",
                    "url": "http://textures.minecraft.net/texture/cape-old",
                    "alias": "MineCon"
                }
            ]
        }));
        let base = spawn_mock(state).await;
        let auth = auth(&base);

        let profile = auth
            .profile("minecraft-access", &CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(
            profile.skin_url(),
            Some("http://textures.minecraft.net/texture/new")
        );
        assert_eq!(profile.skins().len(), 2);
        let active = profile.active_skin().unwrap();
        assert_eq!(active.id(), "skin-active");
        assert_eq!(active.variant(), SkinVariant::Slim);
        assert_eq!(active.alias(), Some("My Skin"));
        let inactive = &profile.skins()[1];
        assert!(!inactive.is_active());
        assert_eq!(inactive.variant(), SkinVariant::Classic);

        assert_eq!(profile.capes().len(), 2);
        assert_eq!(profile.capes()[0].alias(), Some("Migrator"));
        assert!(profile.capes()[0].is_active());
        assert!(!profile.capes()[1].is_active());
    }

    #[tokio::test]
    async fn profile_404_is_reported_as_missing_profile() {
        let state = MockState::new();
        *state.profile.lock().unwrap() = MockReply::status(
            StatusCode::NOT_FOUND,
            json!({
                "path": "/minecraft/profile",
                "error": "NOT_FOUND",
                "errorMessage": "The server has not found anything matching the request URI"
            }),
        );
        let base = spawn_mock(state).await;
        let auth = auth(&base);
        let oauth = MicrosoftOAuthTokens {
            access_token: "msa-access".to_owned(),
            refresh_token: "msa-refresh".to_owned(),
            expires_in: 3600,
        };

        assert!(matches!(
            auth.complete_login(oauth, &CancellationToken::new()).await,
            Err(MicrosoftAuthError::ProfileNotFound)
        ));
    }

    #[tokio::test]
    async fn upload_skin_sends_multipart_and_returns_profile() {
        let state = MockState::new();
        let base = spawn_mock(state.clone()).await;
        let auth = auth(&base);

        let profile = auth
            .upload_skin(
                "minecraft-access",
                SkinVariant::Slim,
                "skin.png",
                b"png-bytes",
                &CancellationToken::new(),
            )
            .await
            .unwrap();

        assert_eq!(
            profile.skin_url(),
            Some("http://textures.minecraft.net/texture/new")
        );
        assert_eq!(profile.active_skin().unwrap().variant(), SkinVariant::Slim);

        let requests = state.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.path, "/minecraft/profile/skins");
        assert_eq!(
            request.authorization.as_deref(),
            Some("Bearer minecraft-access")
        );
        assert!(
            request
                .content_type
                .as_deref()
                .is_some_and(|value| value.starts_with("multipart/form-data"))
        );
        assert!(request.body.contains(r#"name="variant""#));
        assert!(request.body.contains("slim"));
        assert!(request.body.contains(r#"name="file""#));
        assert!(request.body.contains(r#"filename="skin.png""#));
    }

    #[tokio::test]
    async fn change_skin_url_sends_json_and_returns_profile() {
        let state = MockState::new();
        let base = spawn_mock(state.clone()).await;
        let auth = auth(&base);

        auth.change_skin_url(
            "minecraft-access",
            SkinVariant::Classic,
            "https://textures.minecraft.net/texture/old",
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        let requests = state.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.path, "/minecraft/profile/skins");
        assert_eq!(
            request.authorization.as_deref(),
            Some("Bearer minecraft-access")
        );
        assert_eq!(
            json_body(request),
            json!({
                "variant": "classic",
                "url": "https://textures.minecraft.net/texture/old"
            })
        );
    }

    #[tokio::test]
    async fn reset_skin_sends_delete_and_returns_profile() {
        let state = MockState::new();
        let base = spawn_mock(state.clone()).await;
        let auth = auth(&base);

        let profile = auth
            .reset_skin("minecraft-access", &CancellationToken::new())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            profile.skin_url(),
            Some("http://textures.minecraft.net/texture/default")
        );
        let requests = state.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/minecraft/profile/skins/active");
        assert_eq!(
            requests[0].authorization.as_deref(),
            Some("Bearer minecraft-access")
        );
    }

    #[tokio::test]
    async fn skin_upload_maps_rate_limit_and_retry_after() {
        let state = MockState::new();
        *state.skin_change.lock().unwrap() = MockReply::status(
            StatusCode::TOO_MANY_REQUESTS,
            json!({"errorMessage": "Too many requests"}),
        )
        .with_header("retry-after", "30");
        let base = spawn_mock(state).await;
        let auth = auth(&base);

        let error = auth
            .upload_skin(
                "minecraft-access",
                SkinVariant::Classic,
                "skin.png",
                b"png",
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            MicrosoftAuthError::SkinRateLimited {
                retry_after: Some(30)
            }
        ));
    }

    #[tokio::test]
    async fn skin_upload_maps_invalid_skin_message() {
        let state = MockState::new();
        *state.skin_change.lock().unwrap() = MockReply::status(
            StatusCode::BAD_REQUEST,
            json!({"error": "IllegalArgumentException", "errorMessage": "Invalid skin"}),
        );
        let base = spawn_mock(state).await;
        let auth = auth(&base);

        let error = auth
            .upload_skin(
                "minecraft-access",
                SkinVariant::Classic,
                "skin.png",
                b"png",
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            MicrosoftAuthError::SkinRejected { message } if message == "Invalid skin"
        ));
    }

    #[tokio::test]
    async fn skin_upload_maps_suspended_account() {
        let state = MockState::new();
        *state.skin_change.lock().unwrap() = MockReply::status(
            StatusCode::FORBIDDEN,
            json!({
                "errorMessage": "Your account has been suspended.",
                "details": {"reason": "ACCOUNT_SUSPENDED"}
            }),
        );
        let base = spawn_mock(state).await;
        let auth = auth(&base);

        let error = auth
            .upload_skin(
                "minecraft-access",
                SkinVariant::Classic,
                "skin.png",
                b"png",
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            MicrosoftAuthError::SkinForbidden { message } if message.contains("suspended")
        ));
    }

    #[test]
    fn credentials_refresh_before_expiry() {
        let credentials = MicrosoftCredentials::new("refresh", "minecraft", 1_000);

        assert!(credentials.minecraft_token_valid_at(699));
        assert!(!credentials.minecraft_token_valid_at(700));
        assert!(!credentials.minecraft_token_valid_at(1_000));
    }

    #[test]
    fn secret_bearing_types_redact_debug_output() {
        let credentials = MicrosoftCredentials::new("refresh-secret", "minecraft-secret", 1_000);
        let debug = format!("{credentials:?}");
        assert!(!debug.contains("refresh-secret"));
        assert!(!debug.contains("minecraft-secret"));

        let tokens = MicrosoftOAuthTokens {
            access_token: "msa-secret".to_owned(),
            refresh_token: "refresh-secret".to_owned(),
            expires_in: 3600,
        };
        let debug = format!("{tokens:?}");
        assert!(!debug.contains("msa-secret"));
        assert!(!debug.contains("refresh-secret"));

        let token = XboxToken {
            token: "xsts-secret".to_owned(),
            user_hash: "uhs".to_owned(),
        };
        assert!(!format!("{token:?}").contains("xsts-secret"));

        let token = MinecraftToken {
            access_token: "minecraft-secret".to_owned(),
            expires_in: 86400,
        };
        assert!(!format!("{token:?}").contains("minecraft-secret"));
    }
}
