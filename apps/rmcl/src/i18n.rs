//! Interface locale selection and translation lookup.
//!
//! RMCL ships English and Simplified Chinese strings. The locale is resolved
//! once at startup from the stored preference or the operating system, but a
//! settings change applies immediately.

use rmcl_store::LanguagePreference;

pub const ENGLISH: &str = "en";
pub const CHINESE: &str = "zh-CN";

/// Looks up an application string in the active locale.
///
/// Keys live in `locales/ui.yml`. `rust-i18n` falls back to English, so every
/// key must exist in both supported locales to keep the Chinese UI complete.
pub fn tr(key: &str) -> String {
    rust_i18n::t!(key).into_owned()
}

/// Resolves the locale code for a stored preference.
pub fn resolve(preference: LanguagePreference) -> &'static str {
    match preference {
        LanguagePreference::System => system_locale(),
        LanguagePreference::Chinese => CHINESE,
        LanguagePreference::English => ENGLISH,
    }
}

/// Applies a stored preference to the global `rust-i18n` locale.
pub fn apply(preference: LanguagePreference) -> &'static str {
    let locale = resolve(preference);
    rust_i18n::set_locale(locale);
    locale
}

/// Detects the preferred UI language, falling back to English.
///
/// Only Chinese locales are mapped to Simplified Chinese. Regional variants
/// such as `zh-HK` still use the Simplified catalog because RMCL does not ship
/// a separate Traditional Chinese translation.
fn system_locale() -> &'static str {
    match sys_locale::get_locale() {
        Some(locale) if locale.to_ascii_lowercase().starts_with("zh") => CHINESE,
        _ => ENGLISH,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_preferences_resolve_to_supported_locales() {
        assert_eq!(resolve(LanguagePreference::Chinese), CHINESE);
        assert_eq!(resolve(LanguagePreference::English), ENGLISH);
        assert!(matches!(
            resolve(LanguagePreference::System),
            CHINESE | ENGLISH
        ));
    }

    #[test]
    fn only_english_and_simplified_chinese_are_shipped() {
        let mut locales = rust_i18n::available_locales!().to_vec();
        locales.sort_unstable();
        assert_eq!(locales, [ENGLISH, CHINESE]);
    }

    #[test]
    fn representative_application_keys_resolve_in_both_locales() {
        let cases = [
            ("app.page.home.title", "Home", "主页"),
            ("app.page.settings.title", "Settings", "设置"),
            ("app.settings.language", "Language", "语言"),
            ("app.settings.language_system", "Follow System", "跟随系统"),
            (
                "app.create_instance.add_loader_title",
                "Add Mod Loader",
                "添加 Mod 加载器",
            ),
            ("app.worlds.compat_same", "Version matches", "版本一致"),
            ("app.task.phase_running_game", "Game running", "游戏运行中"),
            ("app.skin.tab_online", "Online Skins", "在线皮肤"),
            ("app.content.status_conflict", "Conflict", "冲突"),
        ];
        for (key, english, chinese) in cases {
            rust_i18n::set_locale(ENGLISH);
            assert_eq!(rust_i18n::t!(key).as_ref(), english, "{key} (en)");
            rust_i18n::set_locale(CHINESE);
            assert_eq!(rust_i18n::t!(key).as_ref(), chinese, "{key} (zh-CN)");
        }
        rust_i18n::set_locale(ENGLISH);
    }
}
