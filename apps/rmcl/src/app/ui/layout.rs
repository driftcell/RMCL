//! Shared layout tokens for the RMCL interface.
//!
//! Sizes are expressed in `rem` so they follow the theme's base font size.
//! Pixels stay reserved for physical boundaries: hairlines, window chrome, and
//! raster surfaces.

use gpui_kit::{component::ActiveTheme, *};

/// Width of the persistent navigation sidebar.
pub const SIDEBAR_WIDTH: Rems = rems(14.5);

/// Dialog widths for short confirmations, standard forms, and wide flows.
pub const DIALOG_SMALL: Rems = rems(26.25);
pub const DIALOG_MEDIUM: Rems = rems(29.25);
pub const DIALOG_LARGE: Rems = rems(36.25);
pub const DIALOG_XLARGE: Rems = rems(38.75);

/// Shared geometry for dense table-like surfaces.
pub const TABLE_HEADER_HEIGHT: Rems = rems(2.25);
pub const TABLE_ROW_HEIGHT: Rems = rems(4.0);
pub const DOWNLOAD_DATE_COLUMN: Rems = rems(7.0);
pub const DOWNLOAD_STATUS_COLUMN: Rems = rems(5.375);
pub const DOWNLOAD_ACTION_COLUMN: Rems = rems(6.5);

/// Repeated tile and avatar sizes.
pub const AVATAR_SMALL: Rems = rems(2.0);
pub const AVATAR_MEDIUM: Rems = rems(2.25);
pub const AVATAR_LARGE: Rems = rems(2.5);
pub const TILE_LARGE: Rems = rems(2.75);
pub const HERO_TILE: Rems = rems(5.5);

/// Popup geometry that component APIs take as pixels.
pub const MENU_WIDTH: Rems = rems(17.5);
pub const MENU_MAX_HEIGHT: Rems = rems(16.25);
pub const SELECT_MENU_WIDTH: Rems = rems(12.5);

/// Skin stage dimensions and the viewport width that switches to the wide stage.
pub const SKIN_STAGE: Rems = rems(25.0);
pub const SKIN_STAGE_WIDE: Rems = rems(31.25);
pub const SKIN_STAGE_WIDE_MIN_VIEWPORT: Rems = rems(71.25);

/// Resolves a rem length for component APIs that require `Pixels`.
pub fn rem_px(cx: &App, length: Rems) -> Pixels {
    length.to_pixels(cx.theme().font_size)
}
