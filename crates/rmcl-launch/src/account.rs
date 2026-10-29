use std::{fmt, path::PathBuf};

use uuid::Uuid;

/// JVM configuration for authlib-injector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthlibLaunchConfig {
    pub jar_path: PathBuf,
    pub api_root: String,
    /// Base64 of the exact API root metadata response.
    pub prefetched_metadata: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct LaunchAccount {
    pub username: String,
    pub uuid: Uuid,
    pub access_token: String,
    pub user_type: String,
    pub user_properties: String,
}

impl fmt::Debug for LaunchAccount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LaunchAccount")
            .field("username", &self.username)
            .field("uuid", &self.uuid)
            .field("access_token", &"<redacted>")
            .field("user_type", &self.user_type)
            .field("user_properties", &self.user_properties)
            .finish()
    }
}

impl LaunchAccount {
    /// Creates a launch identity with a fresh random token on every call.
    pub fn from_offline(account: &rmcl_auth::OfflineAccount) -> Self {
        Self {
            username: account.username().to_owned(),
            uuid: account.uuid(),
            access_token: Uuid::new_v4().simple().to_string(),
            user_type: "msa".to_owned(),
            user_properties: "{}".to_owned(),
        }
    }

    /// Creates a launch identity from a verified Microsoft profile and token.
    pub fn from_microsoft(
        account: &rmcl_auth::MicrosoftAccount,
        access_token: impl Into<String>,
    ) -> Self {
        Self {
            username: account.username().to_owned(),
            uuid: account.uuid(),
            access_token: access_token.into(),
            user_type: "msa".to_owned(),
            user_properties: "{}".to_owned(),
        }
    }

    /// Creates a launch identity from a third-party Yggdrasil profile.
    ///
    /// authlib-injector normalizes `mojang` to `msa`; RMCL passes `msa`
    /// directly to match the HMCL baseline and current Minecraft versions.
    pub fn from_authlib(
        account: &rmcl_auth::AuthlibAccount,
        access_token: impl Into<String>,
        user_properties: impl Into<String>,
    ) -> Self {
        Self {
            username: account.username().to_owned(),
            uuid: account.uuid(),
            access_token: access_token.into(),
            user_type: "msa".to_owned(),
            user_properties: user_properties.into(),
        }
    }
}
