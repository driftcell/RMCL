use md5::{Digest, Md5};
use thiserror::Error;
use uuid::Uuid;

/// An offline Minecraft profile with a stable identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OfflineAccount {
    username: String,
    uuid: Uuid,
}

impl OfflineAccount {
    /// Creates an account whose UUID follows the offline-player convention.
    pub fn new(username: impl Into<String>) -> Result<Self, OfflineAccountError> {
        let username = username.into();
        reject_blank_username(&username)?;
        let uuid = offline_player_uuid(&username);
        Ok(Self { username, uuid })
    }

    /// Creates an account with an explicitly stored or user-provided UUID.
    pub fn with_uuid(username: impl Into<String>, uuid: Uuid) -> Result<Self, OfflineAccountError> {
        let username = username.into();
        reject_blank_username(&username)?;
        Ok(Self { username, uuid })
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    /// Returns the compact UUID form used by Minecraft launch arguments.
    pub fn compact_uuid(&self) -> String {
        self.uuid.simple().to_string()
    }

    /// Reports whether the name matches the conventional offline-name policy.
    ///
    /// Nonconforming names remain valid accounts; the UI can use this to show a warning.
    pub fn has_conventional_username(&self) -> bool {
        self.username.len() <= 16
            && self
                .username
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum OfflineAccountError {
    #[error("profile name cannot be blank")]
    BlankProfileName,
}

/// Derives a version-3 UUID from the exact UTF-8 bytes of `OfflinePlayer:<username>`.
pub fn offline_player_uuid(username: &str) -> Uuid {
    // NOTE: Trimming, case folding, or Unicode normalization would change the offline identity.
    let digest = Md5::digest(format!("OfflinePlayer:{username}").as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest);
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn reject_blank_username(username: &str) -> Result<(), OfflineAccountError> {
    if username.trim().is_empty() {
        Err(OfflineAccountError::BlankProfileName)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_known_steve_uuid() {
        let account = OfflineAccount::new("Steve").unwrap();

        assert_eq!(
            account.uuid(),
            Uuid::parse_str("5627dd98-e6be-3c21-b8a8-e92344183641").unwrap()
        );
    }

    #[test]
    fn derives_known_alex_uuid() {
        let account = OfflineAccount::new("Alex").unwrap();

        assert_eq!(
            account.uuid(),
            Uuid::parse_str("36532b5e-c442-3dbb-a24c-c7e55d0f979a").unwrap()
        );
    }

    #[test]
    fn derivation_is_deterministic() {
        assert_eq!(offline_player_uuid("Player"), offline_player_uuid("Player"));
    }

    #[test]
    fn preserves_username_without_normalization() {
        let plain = OfflineAccount::new("Steve").unwrap();
        let trailing_space = OfflineAccount::new("Steve ").unwrap();
        let different_case = OfflineAccount::new("steve").unwrap();

        assert_eq!(trailing_space.username(), "Steve ");
        assert_ne!(plain.uuid(), trailing_space.uuid());
        assert_ne!(plain.uuid(), different_case.uuid());
    }

    #[test]
    fn rejects_blank_usernames() {
        for username in ["", " ", "\t\r\n"] {
            assert_eq!(
                OfflineAccount::new(username),
                Err(OfflineAccountError::BlankProfileName)
            );
        }
    }

    #[test]
    fn accepts_explicit_uuid() {
        let uuid = Uuid::parse_str("123e4567-e89b-12d3-a456-426614174000").unwrap();
        let account = OfflineAccount::with_uuid("Custom", uuid).unwrap();

        assert_eq!(account.uuid(), uuid);
        assert_eq!(account.compact_uuid(), "123e4567e89b12d3a456426614174000");
    }

    #[test]
    fn reports_conventional_usernames() {
        assert!(
            OfflineAccount::new("Steve_2026")
                .unwrap()
                .has_conventional_username()
        );
        assert!(
            OfflineAccount::new("1234567890123456")
                .unwrap()
                .has_conventional_username()
        );
    }

    #[test]
    fn reports_nonconforming_usernames_without_rejecting_them() {
        for username in ["12345678901234567", "with space", "史蒂夫"] {
            let account = OfflineAccount::new(username).unwrap();
            assert!(!account.has_conventional_username());
        }
    }
}
