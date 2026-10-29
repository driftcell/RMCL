//! Platform integration helpers.
//!
//! This crate contains operating-system adapters that are independent of the
//! launcher UI and of Minecraft-specific behavior.

mod process;
mod secret;

pub use process::HideConsoleWindow;
pub use secret::{KeyringSecretStore, MemorySecretStore, SecretStore, SecretStoreError};
