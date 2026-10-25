//! Platform integration helpers.
//!
//! This crate contains operating-system adapters that are independent of the
//! launcher UI and of Minecraft-specific behavior.

mod secret;

pub use secret::{KeyringSecretStore, MemorySecretStore, SecretStore, SecretStoreError};
