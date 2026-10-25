//! Minecraft repository layout and safe repository-relative paths.

mod layout;
mod path;

pub use layout::MinecraftLayout;
pub use path::{LayoutError, RepositoryPath, VersionId};
