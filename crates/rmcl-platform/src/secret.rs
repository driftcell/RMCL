//! Secret storage backed by the operating system credential store.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use thiserror::Error;

/// Stores opaque secret strings outside the launcher configuration.
///
/// Implementations must keep values confidential at rest. Keys are stable
/// account identifiers chosen by the caller and never contain secret data.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, SecretStoreError>;
    fn set(&self, key: &str, value: &str) -> Result<(), SecretStoreError>;
    fn delete(&self, key: &str) -> Result<(), SecretStoreError>;
}

/// System credential store adapter.
///
/// On macOS this uses Keychain, on Windows Credential Manager, and on other
/// Unix systems the Secret Service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyringSecretStore {
    service: String,
}

impl KeyringSecretStore {
    pub fn new(service: impl Into<String>) -> Result<Self, SecretStoreError> {
        let service = service.into();
        validate_key(&service)?;
        Ok(Self { service })
    }

    pub fn service(&self) -> &str {
        &self.service
    }
}

impl SecretStore for KeyringSecretStore {
    fn get(&self, key: &str) -> Result<Option<String>, SecretStoreError> {
        validate_key(key)?;
        let entry = keyring::Entry::new(&self.service, key)?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(SecretStoreError::Keyring(error)),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<(), SecretStoreError> {
        validate_key(key)?;
        let entry = keyring::Entry::new(&self.service, key)?;
        entry.set_password(value)?;
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
        validate_key(key)?;
        let entry = keyring::Entry::new(&self.service, key)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(SecretStoreError::Keyring(error)),
        }
    }
}

/// In-memory secret store for tests and explicitly ephemeral sessions.
#[derive(Debug, Clone, Default)]
pub struct MemorySecretStore {
    values: Arc<Mutex<BTreeMap<String, String>>>,
}

impl SecretStore for MemorySecretStore {
    fn get(&self, key: &str) -> Result<Option<String>, SecretStoreError> {
        validate_key(key)?;
        let values = self.values.lock().map_err(|_| SecretStoreError::Poisoned)?;
        Ok(values.get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<(), SecretStoreError> {
        validate_key(key)?;
        let mut values = self.values.lock().map_err(|_| SecretStoreError::Poisoned)?;
        values.insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
        validate_key(key)?;
        let mut values = self.values.lock().map_err(|_| SecretStoreError::Poisoned)?;
        values.remove(key);
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum SecretStoreError {
    #[error("secret key cannot be blank")]
    BlankKey,
    #[error("system credential storage is unavailable: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("secret store lock is poisoned")]
    Poisoned,
}

fn validate_key(key: &str) -> Result<(), SecretStoreError> {
    if key.trim().is_empty() {
        Err(SecretStoreError::BlankKey)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trips_and_deletes() {
        let store = MemorySecretStore::default();

        assert_eq!(store.get("account").unwrap(), None);
        store.set("account", "secret").unwrap();
        assert_eq!(store.get("account").unwrap().as_deref(), Some("secret"));
        store.set("account", "rotated").unwrap();
        assert_eq!(store.get("account").unwrap().as_deref(), Some("rotated"));
        store.delete("account").unwrap();
        assert_eq!(store.get("account").unwrap(), None);
        store.delete("account").unwrap();
    }

    #[test]
    fn blank_keys_are_rejected() {
        let store = MemorySecretStore::default();

        assert!(matches!(store.get(" "), Err(SecretStoreError::BlankKey)));
        assert!(matches!(
            store.set("", "secret"),
            Err(SecretStoreError::BlankKey)
        ));
    }

    #[test]
    fn keyring_service_requires_a_name() {
        assert!(matches!(
            KeyringSecretStore::new(""),
            Err(SecretStoreError::BlankKey)
        ));
    }

    /// Manual check for the host credential store. Run with:
    /// `cargo test -p rmcl-platform keyring_round_trip -- --ignored`
    #[test]
    #[ignore = "writes to the host credential store"]
    fn keyring_round_trip() {
        let service = "com.rmcl.launcher.test";
        let key = format!("round-trip-{}", std::process::id());
        let store = KeyringSecretStore::new(service).unwrap();

        store.delete(&key).unwrap();
        store.set(&key, "secret-value").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("secret-value"));
        store.delete(&key).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
    }
}
