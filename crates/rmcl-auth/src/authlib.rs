//! Third-party Yggdrasil authentication compatible with authlib-injector.
//!
//! The client implements API root discovery (ALI), server metadata, the
//! Yggdrasil authentication endpoints, and the credential refresh flow. It does
//! not launch Java or persist credentials.

use std::{collections::BTreeMap, fmt, time::Duration};

use reqwest::{
    Client, RequestBuilder, Response, StatusCode,
    header::{ACCEPT, CONTENT_TYPE},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

const METADATA_MAX_BYTES: usize = 256 * 1024;
const RESPONSE_MAX_BYTES: usize = 1024 * 1024;
const ALI_MAX_REDIRECTS: usize = 5;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const USER_AGENT: &str = concat!("RMCL/", env!("CARGO_PKG_VERSION"));
const ALI_HEADER: &str = "x-authlib-injector-api-location";

/// A curated third-party authentication server that can be offered without
/// asking the player to type an API URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthlibServerPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub api_root: &'static str,
    pub homepage: &'static str,
    pub description: &'static str,
}

/// Public authlib-injector compatible servers integrated by RMCL.
///
/// The list intentionally contains only well-known public services. Private or
/// self-hosted servers remain available through the custom URL field.
pub const AUTHLIB_SERVER_PRESETS: &[AuthlibServerPreset] = &[
    AuthlibServerPreset {
        id: "littleskin",
        name: "LittleSkin",
        api_root: "https://littleskin.cn/api/yggdrasil/",
        homepage: "https://littleskin.cn/",
        description: "国内常用皮肤站与外置登录服务",
    },
    AuthlibServerPreset {
        id: "redstone",
        name: "红石皮肤站",
        api_root: "https://mcskin.cn/api/yggdrasil/",
        homepage: "https://mcskin.cn/",
        description: "国内皮肤站与外置登录服务",
    },
    AuthlibServerPreset {
        id: "helloskin",
        name: "HelloSkin",
        api_root: "https://helloskin.cn/api/yggdrasil/",
        homepage: "https://helloskin.cn/",
        description: "国内皮肤站与外置登录服务",
    },
    AuthlibServerPreset {
        id: "mua",
        name: "MUA 用户中心",
        api_root: "https://skin.mualliance.ltd/api/yggdrasil/",
        homepage: "https://skin.mualliance.ltd/",
        description: "Minecraft 高校联盟用户中心",
    },
    AuthlibServerPreset {
        id: "elyby",
        name: "Ely.by",
        api_root: "https://account.ely.by/api/authlib-injector/",
        homepage: "https://ely.by/",
        description: "国际第三方认证服务",
    },
];

/// Returns a preset by its stable ID.
pub fn authlib_server_preset(id: &str) -> Option<&'static AuthlibServerPreset> {
    AUTHLIB_SERVER_PRESETS.iter().find(|preset| preset.id == id)
}

/// Metadata returned by the authentication server API root.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct AuthlibMetadata {
    #[serde(rename = "skinDomains")]
    pub skin_domains: Vec<String>,
    #[serde(rename = "signaturePublickey")]
    pub signature_public_key: Option<String>,
    pub meta: AuthlibMetadataInfo,
}

/// Human-facing server information and optional feature flags.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct AuthlibMetadataInfo {
    #[serde(rename = "serverName")]
    pub server_name: Option<String>,
    pub links: BTreeMap<String, String>,
    pub feature: AuthlibFeature,
    /// Some older Blessing Skin deployments emitted this flat key.
    #[serde(rename = "feature.non_email_login")]
    pub non_email_login_legacy: Option<bool>,
}

/// Optional capabilities advertised by the authentication server.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct AuthlibFeature {
    pub non_email_login: bool,
    #[serde(rename = "legacy_skin_api")]
    pub legacy_skin_api: bool,
    #[serde(rename = "no_mojang_namespace")]
    pub no_mojang_namespace: bool,
    #[serde(rename = "enable_profile_key")]
    pub enable_profile_key: bool,
    #[serde(rename = "enable_mojang_anti_features")]
    pub enable_mojang_anti_features: bool,
    #[serde(rename = "username_check")]
    pub username_check: bool,
}

impl AuthlibMetadata {
    /// Returns whether the server expects a non-email login identifier.
    pub fn non_email_login(&self) -> bool {
        self.meta.feature.non_email_login || self.meta.non_email_login_legacy.unwrap_or(false)
    }

    pub fn server_name(&self) -> Option<&str> {
        self.meta.server_name.as_deref()
    }

    pub fn homepage(&self) -> Option<&str> {
        self.meta.links.get("homepage").map(String::as_str)
    }

    pub fn register_url(&self) -> Option<&str> {
        self.meta.links.get("register").map(String::as_str)
    }
}

/// A resolved authentication server with its raw metadata response.
///
/// The raw response is preserved because authlib-injector prefetch requires the
/// exact bytes returned by the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthlibServer {
    api_root: Url,
    metadata: AuthlibMetadata,
    metadata_raw: String,
}

impl AuthlibServer {
    /// Builds a server from a canonical API root and a raw metadata response.
    pub fn from_metadata(api_root: Url, metadata_raw: String) -> Result<Self, AuthlibError> {
        let api_root = normalize_api_root_url(api_root)?;
        let metadata: AuthlibMetadata =
            serde_json::from_str(&metadata_raw).map_err(AuthlibError::MetadataJson)?;
        Ok(Self {
            api_root,
            metadata,
            metadata_raw,
        })
    }

    pub fn api_root(&self) -> &str {
        self.api_root.as_str()
    }

    pub fn api_root_url(&self) -> &Url {
        &self.api_root
    }

    pub fn metadata(&self) -> &AuthlibMetadata {
        &self.metadata
    }

    pub fn metadata_raw(&self) -> &str {
        &self.metadata_raw
    }

    pub fn display_name(&self) -> &str {
        self.metadata
            .server_name()
            .or_else(|| self.api_root.host_str())
            .unwrap_or("外部认证服务器")
    }
}

/// One player profile on a third-party authentication server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthlibProfile {
    id: Uuid,
    name: String,
}

impl AuthlibProfile {
    pub fn new(id: Uuid, name: impl Into<String>) -> Result<Self, AuthlibError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(AuthlibError::InvalidResponse("profile name is blank"));
        }
        Ok(Self { id, name })
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn compact_id(&self) -> String {
        self.id.simple().to_string()
    }
}

#[derive(Deserialize)]
struct RawProfile {
    id: String,
    name: String,
}

impl TryFrom<RawProfile> for AuthlibProfile {
    type Error = String;

    fn try_from(raw: RawProfile) -> Result<Self, Self::Error> {
        let id = Uuid::parse_str(&raw.id).map_err(|error| error.to_string())?;
        Self::new(id, raw.name).map_err(|error| error.to_string())
    }
}

impl<'de> Deserialize<'de> for AuthlibProfile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        RawProfile::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

/// A stable launcher account bound to one authentication server.
///
/// `id` is generated by RMCL and remains stable when the player name changes.
/// The profile UUID alone is not sufficient because different servers may use
/// the same UUID for different accounts.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AuthlibAccount {
    id: Uuid,
    username: String,
    uuid: Uuid,
    api_root: String,
    server_name: Option<String>,
}

impl AuthlibAccount {
    pub fn new(
        id: Uuid,
        username: impl Into<String>,
        uuid: Uuid,
        api_root: impl Into<String>,
        server_name: Option<String>,
    ) -> Result<Self, AuthlibAccountError> {
        let username = username.into();
        let api_root = api_root.into();
        if username.trim().is_empty() {
            return Err(AuthlibAccountError::BlankUsername);
        }
        if api_root.trim().is_empty() {
            return Err(AuthlibAccountError::BlankApiRoot);
        }
        Ok(Self {
            id,
            username,
            uuid,
            api_root,
            server_name,
        })
    }

    pub fn from_profile(
        id: Uuid,
        profile: &AuthlibProfile,
        server: &AuthlibServer,
    ) -> Result<Self, AuthlibAccountError> {
        Self::new(
            id,
            profile.name(),
            profile.id(),
            server.api_root(),
            Some(server.display_name().to_owned()),
        )
    }

    pub fn id(&self) -> Uuid {
        self.id
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

    pub fn api_root(&self) -> &str {
        &self.api_root
    }

    pub fn server_name(&self) -> Option<&str> {
        self.server_name.as_deref()
    }

    /// Updates the cached profile name and server display name after refresh.
    pub fn update_profile(&mut self, profile: &AuthlibProfile, server: &AuthlibServer) {
        self.username = profile.name().to_owned();
        self.server_name = Some(server.display_name().to_owned());
    }
}

/// Credentials persisted in the operating system secret store.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthlibCredentials {
    pub version: u32,
    pub access_token: String,
    pub client_token: String,
    /// Minecraft `--userProperties` JSON produced from the Yggdrasil user.
    #[serde(default = "empty_user_properties")]
    pub user_properties: String,
}

impl fmt::Debug for AuthlibCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthlibCredentials")
            .field("version", &self.version)
            .field("access_token", &"<redacted>")
            .field("client_token", &"<redacted>")
            .field("user_properties", &self.user_properties)
            .finish()
    }
}

impl AuthlibCredentials {
    pub const SCHEMA_VERSION: u32 = 1;

    pub fn new(
        access_token: impl Into<String>,
        client_token: impl Into<String>,
        user_properties: impl Into<String>,
    ) -> Self {
        Self {
            version: Self::SCHEMA_VERSION,
            access_token: access_token.into(),
            client_token: client_token.into(),
            user_properties: user_properties.into(),
        }
    }
}

fn empty_user_properties() -> String {
    "{}".to_owned()
}

/// The result of a successful authentication or refresh request.
#[derive(Debug, Clone)]
pub struct AuthlibLogin {
    server: AuthlibServer,
    credentials: AuthlibCredentials,
    selected_profile: Option<AuthlibProfile>,
    available_profiles: Vec<AuthlibProfile>,
}

impl AuthlibLogin {
    pub fn server(&self) -> &AuthlibServer {
        &self.server
    }

    pub fn credentials(&self) -> &AuthlibCredentials {
        &self.credentials
    }

    pub fn selected_profile(&self) -> Option<&AuthlibProfile> {
        self.selected_profile.as_ref()
    }

    pub fn available_profiles(&self) -> &[AuthlibProfile] {
        &self.available_profiles
    }

    /// Returns the profile that should be used, or `None` when the player must
    /// choose from multiple available profiles.
    pub fn single_profile(&self) -> Option<&AuthlibProfile> {
        self.selected_profile
            .as_ref()
            .or_else(|| (self.available_profiles.len() == 1).then(|| &self.available_profiles[0]))
    }
}

/// A headless Yggdrasil client with injectable HTTP behavior for tests.
#[derive(Clone)]
pub struct AuthlibClient {
    metadata_client: Client,
    api_client: Client,
}

impl fmt::Debug for AuthlibClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthlibClient")
            .finish_non_exhaustive()
    }
}

impl AuthlibClient {
    pub fn new() -> Result<Self, AuthlibError> {
        let metadata_client = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(AuthlibError::Client)?;
        let api_client = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            // Authentication requests carry passwords. A redirect must never
            // turn a POST into a GET or forward credentials to another origin.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(AuthlibError::Client)?;
        Ok(Self {
            metadata_client,
            api_client,
        })
    }

    /// Builds a client with explicit HTTP clients. Intended for tests and
    /// distributions with a custom transport policy.
    pub fn with_clients(metadata_client: Client, api_client: Client) -> Self {
        Self {
            metadata_client,
            api_client,
        }
    }

    /// Resolves user input to a canonical API root and fetches metadata.
    ///
    /// Missing schemes default to HTTPS. Explicit HTTP input is accepted, but
    /// an HTTPS URL is never downgraded to HTTP by a redirect or ALI hop.
    pub async fn resolve_server(
        &self,
        input: &str,
        cancellation: &CancellationToken,
    ) -> Result<AuthlibServer, AuthlibError> {
        let mut url = normalize_api_root(input)?;
        let mut visited = Vec::new();
        for _ in 0..=ALI_MAX_REDIRECTS {
            if visited.iter().any(|visited| same_api_root(visited, &url)) {
                return Err(AuthlibError::AliLoop);
            }
            visited.push(url.clone());

            let request = self
                .metadata_client
                .get(url.clone())
                .header(ACCEPT, "application/json");
            let response = self
                .send(request, "fetch authlib-injector metadata", cancellation)
                .await?;
            let status = response.status();
            let final_url = response.url().clone();
            ensure_no_downgrade(&url, &final_url)?;
            let ali = response
                .headers()
                .get(ALI_HEADER)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let body = read_body_limited(
                response,
                METADATA_MAX_BYTES,
                "fetch authlib-injector metadata",
                cancellation,
            )
            .await?;

            if let Some(ali) = ali {
                let next = final_url
                    .join(&ali)
                    .map_err(|error| AuthlibError::InvalidServerUrl(error.to_string()))?;
                let next = normalize_api_root_url(next)?;
                if !same_api_root(&final_url, &next) {
                    ensure_no_downgrade(&final_url, &next)?;
                    url = next;
                    continue;
                }
            }

            if !status.is_success() {
                return Err(http_status_error(
                    "fetch authlib-injector metadata",
                    status,
                    &body,
                ));
            }
            return AuthlibServer::from_metadata(final_url, body);
        }
        Err(AuthlibError::AliTooManyRedirects)
    }

    /// Authenticates a username and password.
    ///
    /// The returned login may not have a selected profile when the account owns
    /// multiple characters. Call [`Self::refresh`] with the chosen profile.
    pub async fn authenticate(
        &self,
        server: &AuthlibServer,
        username: &str,
        password: &str,
        client_token: Uuid,
        cancellation: &CancellationToken,
    ) -> Result<AuthlibLogin, AuthlibError> {
        if username.trim().is_empty() {
            return Err(AuthlibError::InvalidUsername);
        }
        if password.is_empty() {
            return Err(AuthlibError::InvalidPassword);
        }
        let client_token = client_token.simple().to_string();
        let body = json!({
            "agent": { "name": "Minecraft", "version": 1 },
            "username": username,
            "password": password,
            "clientToken": client_token,
            "requestUser": true,
        });
        let (status, body) = self
            .post_json(
                auth_endpoint(server, "authenticate")?,
                &body,
                "authenticate with authlib server",
                cancellation,
            )
            .await?;
        parse_login_response(
            server,
            status,
            &body,
            &client_token,
            "authenticate with authlib server",
        )
    }

    /// Refreshes an access token, optionally selecting a specific profile.
    pub async fn refresh(
        &self,
        server: &AuthlibServer,
        credentials: &AuthlibCredentials,
        selected_profile: Option<&AuthlibProfile>,
        cancellation: &CancellationToken,
    ) -> Result<AuthlibLogin, AuthlibError> {
        let mut body = Map::new();
        body.insert(
            "accessToken".to_owned(),
            Value::String(credentials.access_token.clone()),
        );
        body.insert(
            "clientToken".to_owned(),
            Value::String(credentials.client_token.clone()),
        );
        body.insert("requestUser".to_owned(), Value::Bool(true));
        if let Some(profile) = selected_profile {
            body.insert(
                "selectedProfile".to_owned(),
                json!({
                    "id": profile.compact_id(),
                    "name": profile.name(),
                }),
            );
        }
        let (status, body) = self
            .post_json(
                auth_endpoint(server, "refresh")?,
                &Value::Object(body),
                "refresh authlib session",
                cancellation,
            )
            .await?;
        let login = parse_login_response(
            server,
            status,
            &body,
            &credentials.client_token,
            "refresh authlib session",
        )?;
        if let Some(expected) = selected_profile {
            let selected = login
                .selected_profile
                .as_ref()
                .ok_or(AuthlibError::ProfileMismatch)?;
            if selected.id() != expected.id() {
                return Err(AuthlibError::ProfileMismatch);
            }
        }
        Ok(login)
    }

    /// Returns whether a token is currently accepted by the server.
    pub async fn validate(
        &self,
        server: &AuthlibServer,
        credentials: &AuthlibCredentials,
        cancellation: &CancellationToken,
    ) -> Result<bool, AuthlibError> {
        let body = json!({
            "accessToken": credentials.access_token,
            "clientToken": credentials.client_token,
        });
        let (status, body) = self
            .post_json(
                auth_endpoint(server, "validate")?,
                &body,
                "validate authlib session",
                cancellation,
            )
            .await?;
        if status.is_success() {
            if let Some(code) = remote_error_code(&body) {
                return if code == "ForbiddenOperationException" {
                    Ok(false)
                } else {
                    Err(AuthlibError::Remote {
                        code,
                        message: remote_error_message(&body),
                    })
                };
            }
            return Ok(true);
        }
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Ok(false);
        }
        Err(http_status_error("validate authlib session", status, &body))
    }

    /// Best-effort invalidation used when removing an account.
    pub async fn invalidate(
        &self,
        server: &AuthlibServer,
        credentials: &AuthlibCredentials,
        cancellation: &CancellationToken,
    ) -> Result<(), AuthlibError> {
        let body = json!({
            "accessToken": credentials.access_token,
            "clientToken": credentials.client_token,
        });
        let (status, body) = self
            .post_json(
                auth_endpoint(server, "invalidate")?,
                &body,
                "invalidate authlib session",
                cancellation,
            )
            .await?;
        if status.is_success() || matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
        {
            return Ok(());
        }
        Err(http_status_error(
            "invalidate authlib session",
            status,
            &body,
        ))
    }

    async fn post_json(
        &self,
        url: Url,
        body: &impl Serialize,
        operation: &'static str,
        cancellation: &CancellationToken,
    ) -> Result<(StatusCode, String), AuthlibError> {
        let request = self
            .api_client
            .post(url)
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(body);
        let response = self.send(request, operation, cancellation).await?;
        let status = response.status();
        let body = read_body_limited(response, RESPONSE_MAX_BYTES, operation, cancellation).await?;
        Ok((status, body))
    }

    async fn send(
        &self,
        request: RequestBuilder,
        operation: &'static str,
        cancellation: &CancellationToken,
    ) -> Result<Response, AuthlibError> {
        tokio::select! {
            _ = cancellation.cancelled() => Err(AuthlibError::Cancelled),
            result = request.send() => result.map_err(|source| AuthlibError::Http { operation, source }),
        }
    }
}

fn auth_endpoint(server: &AuthlibServer, endpoint: &str) -> Result<Url, AuthlibError> {
    server
        .api_root_url()
        .join(&format!("authserver/{endpoint}"))
        .map_err(|error| AuthlibError::InvalidServerUrl(error.to_string()))
}

async fn read_body_limited(
    mut response: Response,
    limit: usize,
    operation: &'static str,
    cancellation: &CancellationToken,
) -> Result<String, AuthlibError> {
    if let Some(length) = response.content_length()
        && length > limit as u64
    {
        return Err(AuthlibError::ResponseTooLarge { limit });
    }
    let mut bytes = Vec::new();
    loop {
        let chunk = tokio::select! {
            _ = cancellation.cancelled() => return Err(AuthlibError::Cancelled),
            chunk = response.chunk() => chunk.map_err(|source| AuthlibError::Http { operation, source })?,
        };
        let Some(chunk) = chunk else {
            break;
        };
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(AuthlibError::ResponseTooLarge { limit });
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|_| AuthlibError::InvalidUtf8)
}

#[derive(Deserialize)]
struct RawAuthResponse {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
    #[serde(rename = "clientToken")]
    client_token: Option<String>,
    #[serde(rename = "selectedProfile")]
    selected_profile: Option<AuthlibProfile>,
    #[serde(rename = "availableProfiles", default)]
    available_profiles: Option<Vec<AuthlibProfile>>,
    #[serde(default)]
    user: Option<RawUser>,
    #[serde(default)]
    error: Option<String>,
    #[serde(rename = "errorMessage", default)]
    error_message: Option<String>,
}

#[derive(Deserialize)]
struct RawUser {
    #[serde(default)]
    properties: Option<Vec<RawUserProperty>>,
}

#[derive(Deserialize)]
struct RawUserProperty {
    name: String,
    value: String,
}

fn parse_login_response(
    server: &AuthlibServer,
    status: StatusCode,
    body: &str,
    request_client_token: &str,
    operation: &'static str,
) -> Result<AuthlibLogin, AuthlibError> {
    let raw: RawAuthResponse = match serde_json::from_str(body) {
        Ok(raw) => raw,
        Err(_) if !status.is_success() => {
            return Err(http_status_error(operation, status, body));
        }
        Err(_) => {
            return Err(AuthlibError::InvalidResponse(
                "malformed authentication response",
            ));
        }
    };
    if let Some(code) = raw.error.as_deref() {
        return Err(AuthlibError::Remote {
            code: code.to_owned(),
            message: truncate_body(&raw.error_message.unwrap_or_default()),
        });
    }
    if !status.is_success() {
        return Err(http_status_error(operation, status, body));
    }
    let access_token = raw
        .access_token
        .filter(|token| !token.trim().is_empty())
        .ok_or(AuthlibError::InvalidResponse("missing access token"))?;
    let client_token = raw
        .client_token
        .filter(|token| !token.trim().is_empty())
        .ok_or(AuthlibError::InvalidResponse("missing client token"))?;
    if client_token != request_client_token {
        return Err(AuthlibError::ClientTokenMismatch);
    }
    let user_properties = user_properties(raw.user.as_ref());
    Ok(AuthlibLogin {
        server: server.clone(),
        credentials: AuthlibCredentials::new(access_token, client_token, user_properties),
        selected_profile: raw.selected_profile,
        available_profiles: raw.available_profiles.unwrap_or_default(),
    })
}

fn user_properties(user: Option<&RawUser>) -> String {
    let Some(user) = user else {
        return empty_user_properties();
    };
    let mut properties: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for property in user.properties.as_deref().unwrap_or_default() {
        properties
            .entry(property.name.clone())
            .or_default()
            .push(property.value.clone());
    }
    serde_json::to_string(&properties).unwrap_or_else(|_| empty_user_properties())
}

fn remote_error_code(body: &str) -> Option<String> {
    serde_json::from_str::<Value>(body).ok().and_then(|value| {
        value
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_owned)
    })
}

fn remote_error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("errorMessage")
                .and_then(Value::as_str)
                .map(truncate_body)
        })
        .unwrap_or_else(|| truncate_body(body))
}

fn http_status_error(operation: &'static str, status: StatusCode, body: &str) -> AuthlibError {
    if let Some(code) = remote_error_code(body) {
        return AuthlibError::Remote {
            code,
            message: remote_error_message(body),
        };
    }
    AuthlibError::HttpStatus {
        operation,
        status,
        body: truncate_body(body),
    }
}

fn truncate_body(body: &str) -> String {
    body.chars().take(512).collect()
}

/// Normalizes user input into a canonical API root URL.
pub fn normalize_api_root(input: &str) -> Result<Url, AuthlibError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(AuthlibError::InvalidServerUrl(
            "authentication server URL cannot be blank".to_owned(),
        ));
    }
    let with_scheme = if input.contains("://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    let url = Url::parse(&with_scheme)
        .map_err(|error| AuthlibError::InvalidServerUrl(error.to_string()))?;
    normalize_api_root_url(url)
}

fn normalize_api_root_url(mut url: Url) -> Result<Url, AuthlibError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AuthlibError::InvalidServerUrl(
            "authentication server URL must use HTTP or HTTPS".to_owned(),
        ));
    }
    if url.host_str().is_none() {
        return Err(AuthlibError::InvalidServerUrl(
            "authentication server URL must include a host".to_owned(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AuthlibError::InvalidServerUrl(
            "authentication server URL must not contain credentials".to_owned(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(AuthlibError::InvalidServerUrl(
            "authentication server URL must not contain a query or fragment".to_owned(),
        ));
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

fn ensure_no_downgrade(from: &Url, to: &Url) -> Result<(), AuthlibError> {
    if from.scheme() == "https" && to.scheme() != "https" {
        Err(AuthlibError::InsecureRedirect)
    } else {
        Ok(())
    }
}

fn same_api_root(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
        && a.path().trim_end_matches('/') == b.path().trim_end_matches('/')
}

#[derive(Debug, Error)]
pub enum AuthlibAccountError {
    #[error("authentication server account name cannot be blank")]
    BlankUsername,
    #[error("authentication server API root cannot be blank")]
    BlankApiRoot,
}

#[derive(Debug, Error)]
pub enum AuthlibError {
    #[error("failed to create the authlib-injector HTTP client: {0}")]
    Client(#[source] reqwest::Error),
    #[error("invalid authentication server URL: {0}")]
    InvalidServerUrl(String),
    #[error("authentication server metadata is not valid JSON: {0}")]
    MetadataJson(#[source] serde_json::Error),
    #[error("authentication server redirected from HTTPS to HTTP")]
    InsecureRedirect,
    #[error("authentication server API location redirects in a loop")]
    AliLoop,
    #[error("authentication server API location redirect limit exceeded")]
    AliTooManyRedirects,
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
    #[error("authentication server error `{code}`: {message}")]
    Remote { code: String, message: String },
    #[error("authentication server response exceeded {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("authentication server returned non-UTF-8 data")]
    InvalidUtf8,
    #[error("authentication server returned an invalid response: {0}")]
    InvalidResponse(&'static str),
    #[error("username cannot be blank")]
    InvalidUsername,
    #[error("password cannot be blank")]
    InvalidPassword,
    #[error("the authentication server changed the client token")]
    ClientTokenMismatch,
    #[error("the authentication server selected a different profile")]
    ProfileMismatch,
    #[error("authentication was cancelled")]
    Cancelled,
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderValue, StatusCode},
        response::IntoResponse,
        routing::{get, post},
    };
    use serde_json::json;

    use super::*;

    fn server(api_root: &str, metadata: &str) -> AuthlibServer {
        AuthlibServer::from_metadata(Url::parse(api_root).unwrap(), metadata.to_owned()).unwrap()
    }

    #[test]
    fn normalizes_https_and_preserves_explicit_http() {
        assert_eq!(
            normalize_api_root("littleskin.cn/api/yggdrasil")
                .unwrap()
                .as_str(),
            "https://littleskin.cn/api/yggdrasil/"
        );
        assert_eq!(
            normalize_api_root("http://127.0.0.1:8080/api/yggdrasil")
                .unwrap()
                .as_str(),
            "http://127.0.0.1:8080/api/yggdrasil/"
        );
        assert!(normalize_api_root("").is_err());
        assert!(normalize_api_root("https://user:pass@example.com/").is_err());
        assert!(normalize_api_root("https://example.com/?query=1").is_err());
    }

    #[test]
    fn https_is_never_downgraded_to_http() {
        let https = Url::parse("https://example.com/api/yggdrasil/").unwrap();
        let http = Url::parse("http://example.com/api/yggdrasil/").unwrap();
        assert!(matches!(
            ensure_no_downgrade(&https, &http),
            Err(AuthlibError::InsecureRedirect)
        ));
        assert!(ensure_no_downgrade(&http, &https).is_ok());
    }

    #[test]
    fn metadata_exposes_server_name_and_links() {
        let metadata = server(
            "https://example.com/api/yggdrasil/",
            r#"{
                "meta": {
                    "serverName": "Example Skin",
                    "links": {"homepage": "https://example.com", "register": "https://example.com/register"},
                    "feature": {"non_email_login": true}
                },
                "skinDomains": ["example.com"]
            }"#,
        );
        assert_eq!(metadata.display_name(), "Example Skin");
        assert_eq!(metadata.metadata().homepage(), Some("https://example.com"));
        assert_eq!(
            metadata.metadata().register_url(),
            Some("https://example.com/register")
        );
        assert!(metadata.metadata().non_email_login());
        assert_eq!(metadata.metadata().skin_domains, ["example.com"]);
    }

    #[test]
    fn credentials_debug_redacts_tokens() {
        let credentials = AuthlibCredentials::new("access-secret", "client-secret", "{}");
        let debug = format!("{credentials:?}");
        assert!(!debug.contains("access-secret"));
        assert!(!debug.contains("client-secret"));
        assert!(debug.contains("<redacted>"));
    }

    #[derive(Clone, Default)]
    struct MockState {
        auth_calls: Arc<AtomicUsize>,
        refresh_calls: Arc<AtomicUsize>,
        validate_calls: Arc<AtomicUsize>,
    }

    async fn spawn_mock() -> (String, MockState) {
        let state = MockState::default();
        let router = Router::new()
            .route("/", get(mock_ali))
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
        (format!("http://{address}/"), state)
    }

    async fn mock_ali() -> impl IntoResponse {
        let mut response = StatusCode::OK.into_response();
        response
            .headers_mut()
            .insert(ALI_HEADER, HeaderValue::from_static("/api/yggdrasil/"));
        response
    }

    async fn mock_metadata() -> impl IntoResponse {
        Json(json!({
            "meta": {"serverName": "Mock Skin", "links": {"homepage": "https://example.invalid"}},
            "skinDomains": ["example.invalid"]
        }))
    }

    async fn mock_authenticate(
        State(state): State<MockState>,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        state.auth_calls.fetch_add(1, Ordering::SeqCst);
        let client_token = body["clientToken"].as_str().unwrap().to_owned();
        if body["password"] != "correct" {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "error": "ForbiddenOperationException",
                    "errorMessage": "Invalid credentials. Invalid username or password."
                })),
            )
                .into_response();
        }
        Json(json!({
            "accessToken": "access-token",
            "clientToken": client_token,
            "selectedProfile": {"id": "5627dd98e6be3c21b8a8e92344183641", "name": "Steve"},
            "availableProfiles": [{"id": "5627dd98e6be3c21b8a8e92344183641", "name": "Steve"}],
            "user": {"id": "user-1", "properties": [{"name": "preferredLanguage", "value": "zh_CN"}]}
        }))
        .into_response()
    }

    async fn mock_refresh(
        State(state): State<MockState>,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        state.refresh_calls.fetch_add(1, Ordering::SeqCst);
        let client_token = body["clientToken"].as_str().unwrap().to_owned();
        Json(json!({
            "accessToken": "refreshed-token",
            "clientToken": client_token,
            "selectedProfile": {"id": "5627dd98e6be3c21b8a8e92344183641", "name": "Steve"},
            "availableProfiles": [{"id": "5627dd98e6be3c21b8a8e92344183641", "name": "Steve"}],
            "user": {"id": "user-1", "properties": []}
        }))
        .into_response()
    }

    async fn mock_validate(
        State(state): State<MockState>,
        Json(_body): Json<Value>,
    ) -> impl IntoResponse {
        state.validate_calls.fetch_add(1, Ordering::SeqCst);
        StatusCode::NO_CONTENT
    }

    #[tokio::test]
    async fn resolves_ali_and_authenticates() {
        let (base, state) = spawn_mock().await;
        let client = AuthlibClient::new().unwrap();
        let cancellation = CancellationToken::new();
        let server = client.resolve_server(&base, &cancellation).await.unwrap();
        assert_eq!(server.display_name(), "Mock Skin");

        let login = client
            .authenticate(&server, "Steve", "correct", Uuid::new_v4(), &cancellation)
            .await
            .unwrap();
        assert_eq!(state.auth_calls.load(Ordering::SeqCst), 1);
        assert_eq!(login.selected_profile().unwrap().name(), "Steve");
        assert_eq!(login.credentials().access_token, "access-token");
        assert_eq!(
            login.credentials().user_properties,
            r#"{"preferredLanguage":["zh_CN"]}"#
        );

        let credentials = login.credentials().clone();
        assert!(
            client
                .validate(&server, &credentials, &cancellation)
                .await
                .unwrap()
        );
        assert_eq!(state.validate_calls.load(Ordering::SeqCst), 1);

        let refreshed = client
            .refresh(
                &server,
                &credentials,
                login.selected_profile(),
                &cancellation,
            )
            .await
            .unwrap();
        assert_eq!(state.refresh_calls.load(Ordering::SeqCst), 1);
        assert_eq!(refreshed.credentials().access_token, "refreshed-token");
    }

    #[tokio::test]
    async fn wrong_password_is_a_remote_error() {
        let (base, _) = spawn_mock().await;
        let client = AuthlibClient::new().unwrap();
        let cancellation = CancellationToken::new();
        let server = client.resolve_server(&base, &cancellation).await.unwrap();
        let error = client
            .authenticate(&server, "Steve", "wrong", Uuid::new_v4(), &cancellation)
            .await
            .unwrap_err();
        match error {
            AuthlibError::Remote { code, message } => {
                assert_eq!(code, "ForbiddenOperationException");
                assert!(message.contains("Invalid credentials"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn client_token_change_is_rejected() {
        let (base, _) = spawn_mock().await;
        let client = AuthlibClient::new().unwrap();
        let cancellation = CancellationToken::new();
        let server = client.resolve_server(&base, &cancellation).await.unwrap();
        // The mock always echoes the request token, so force the response by
        // calling the parser directly with a changed token.
        let error = parse_login_response(
            &server,
            StatusCode::OK,
            r#"{"accessToken":"a","clientToken":"b"}"#,
            "a",
            "authenticate with authlib server",
        )
        .unwrap_err();
        assert!(matches!(error, AuthlibError::ClientTokenMismatch));
    }

    #[test]
    fn presets_are_unique_and_use_http_schemes() {
        let mut ids = std::collections::HashSet::new();
        for preset in AUTHLIB_SERVER_PRESETS {
            assert!(ids.insert(preset.id));
            let url = normalize_api_root(preset.api_root).unwrap();
            assert!(matches!(url.scheme(), "http" | "https"));
        }
    }
}
