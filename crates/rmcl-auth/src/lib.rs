//! Headless authentication primitives for RMCL.

mod account;
mod authlib;
mod microsoft;
mod offline;

pub use account::{Account, AccountKind};
pub use authlib::{
    AUTHLIB_SERVER_PRESETS, AuthlibAccount, AuthlibAccountError, AuthlibClient, AuthlibCredentials,
    AuthlibError, AuthlibFeature, AuthlibLogin, AuthlibMetadata, AuthlibMetadataInfo,
    AuthlibProfile, AuthlibServer, AuthlibServerPreset, authlib_server_preset, normalize_api_root,
};
pub use microsoft::{
    AuthenticatedMicrosoft, DEFAULT_MICROSOFT_CLIENT_ID, DeviceCode, DeviceCodePoll, DeviceLogin,
    MICROSOFT_SCOPE, MINECRAFT_TOKEN_REFRESH_MARGIN_SECS, MicrosoftAccount, MicrosoftAccountError,
    MicrosoftAuth, MicrosoftAuthError, MicrosoftCredentials, MicrosoftEndpoints,
    MicrosoftOAuthTokens, ProfileCape, ProfileSkin, SkinVariant, XboxToken,
};
pub use offline::{OfflineAccount, OfflineAccountError, offline_player_uuid};
