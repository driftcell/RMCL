use std::fmt;

use uuid::Uuid;

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
}
