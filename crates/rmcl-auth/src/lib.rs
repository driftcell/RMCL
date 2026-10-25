//! Headless authentication primitives for RMCL.

mod offline;

pub use offline::{OfflineAccount, OfflineAccountError, offline_player_uuid};
