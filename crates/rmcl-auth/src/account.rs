use uuid::Uuid;

use crate::{AuthlibAccount, MicrosoftAccount, OfflineAccount};

/// A launchable account identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Account {
    Offline(OfflineAccount),
    Microsoft(MicrosoftAccount),
    Authlib(AuthlibAccount),
}

impl Account {
    pub fn username(&self) -> &str {
        match self {
            Self::Offline(account) => account.username(),
            Self::Microsoft(account) => account.username(),
            Self::Authlib(account) => account.username(),
        }
    }

    pub fn uuid(&self) -> Uuid {
        match self {
            Self::Offline(account) => account.uuid(),
            Self::Microsoft(account) => account.uuid(),
            Self::Authlib(account) => account.uuid(),
        }
    }

    pub fn kind(&self) -> AccountKind {
        match self {
            Self::Offline(_) => AccountKind::Offline,
            Self::Microsoft(_) => AccountKind::Microsoft,
            Self::Authlib(_) => AccountKind::Authlib,
        }
    }

    pub fn as_microsoft(&self) -> Option<&MicrosoftAccount> {
        match self {
            Self::Microsoft(account) => Some(account),
            Self::Offline(_) | Self::Authlib(_) => None,
        }
    }

    pub fn as_authlib(&self) -> Option<&AuthlibAccount> {
        match self {
            Self::Authlib(account) => Some(account),
            Self::Offline(_) | Self::Microsoft(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccountKind {
    Offline,
    Microsoft,
    Authlib,
}
