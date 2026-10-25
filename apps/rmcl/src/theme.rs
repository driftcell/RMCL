use gpui_kit::component::{ActiveTheme, Theme, ThemeMode};
use gpui_kit::*;
use rmcl_store::{AccentColor, AppearanceConfig, ThemePreference};

struct Palette {
    primary: Hsla,
    primary_hover: Hsla,
    primary_active: Hsla,
    foreground: Hsla,
}

pub fn apply(appearance: AppearanceConfig, window: Option<&mut Window>, cx: &mut App) {
    match appearance.theme {
        ThemePreference::System => {
            cx.set_window_appearance(None);
            Theme::sync_system_appearance(window, cx);
        }
        ThemePreference::Light => {
            cx.set_window_appearance(Some(WindowAppearance::Light));
            Theme::change(ThemeMode::Light, window, cx);
        }
        ThemePreference::Dark => {
            cx.set_window_appearance(Some(WindowAppearance::Dark));
            Theme::change(ThemeMode::Dark, window, cx);
        }
    }

    let dark = cx.theme().mode.is_dark();
    let palette = palette(appearance.accent, dark);
    Theme::update(cx, |theme| {
        theme.primary = palette.primary;
        theme.primary_hover = palette.primary_hover;
        theme.primary_active = palette.primary_active;
        theme.primary_foreground = palette.foreground;
        theme.button_primary = palette.primary;
        theme.button_primary_hover = palette.primary_hover;
        theme.button_primary_active = palette.primary_active;
        theme.button_primary_foreground = palette.foreground;
        theme.sidebar_primary = palette.primary;
        theme.sidebar_primary_foreground = palette.foreground;
        theme.sidebar_accent = palette.primary.opacity(if dark { 0.22 } else { 0.12 });
        theme.sidebar_accent_foreground = palette.primary;
        theme.ring = palette.primary;
        theme.selection = palette.primary.opacity(0.22);
    });
}

fn palette(accent: AccentColor, dark: bool) -> Palette {
    let (primary, hover, active, foreground) = match (accent, dark) {
        (AccentColor::Emerald, false) => (0x1F8A5B, 0x18744C, 0x12603E, 0xFFFFFF),
        (AccentColor::Emerald, true) => (0x4ADE94, 0x6BE7A8, 0x34C97F, 0x07130C),
        (AccentColor::Ocean, false) => (0x2563EB, 0x1D4ED8, 0x1E40AF, 0xFFFFFF),
        (AccentColor::Ocean, true) => (0x60A5FA, 0x7DB6FF, 0x3B82F6, 0x071120),
        (AccentColor::Violet, false) => (0x7C3AED, 0x6D28D9, 0x5B21B6, 0xFFFFFF),
        (AccentColor::Violet, true) => (0xA78BFA, 0xB9A5FF, 0x8B5CF6, 0x120A24),
        (AccentColor::Amber, false) => (0xB45309, 0x92400E, 0x78350F, 0xFFFFFF),
        (AccentColor::Amber, true) => (0xFBBF24, 0xFCD34D, 0xF59E0B, 0x1A1203),
        (AccentColor::Rose, false) => (0xBE123C, 0x9F1239, 0x881337, 0xFFFFFF),
        (AccentColor::Rose, true) => (0xFB7185, 0xFDA4AF, 0xF43F5E, 0x1F0710),
    };
    Palette {
        primary: rgb(primary).into(),
        primary_hover: rgb(hover).into(),
        primary_active: rgb(active).into(),
        foreground: rgb(foreground).into(),
    }
}
