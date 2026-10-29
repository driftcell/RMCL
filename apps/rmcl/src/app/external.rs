use rmcl_store::LauncherConfig;

/// Applies only external data that MCP can currently modify. Local preferences
/// and editor-owned values stay in the existing configuration.
pub fn merge_external_config(current: &mut LauncherConfig, external: LauncherConfig) -> bool {
    let accounts_changed = current.accounts != external.accounts
        || current.microsoft_accounts != external.microsoft_accounts
        || current.authlib_accounts != external.authlib_accounts
        || current.selected_account != external.selected_account
        || current.selected_account_kind != external.selected_account_kind
        || current.selected_account_id != external.selected_account_id;
    if accounts_changed {
        current.accounts = external.accounts;
        current.microsoft_accounts = external.microsoft_accounts;
        current.authlib_accounts = external.authlib_accounts;
        current.selected_account = external.selected_account;
        current.selected_account_kind = external.selected_account_kind;
        current.selected_account_id = external.selected_account_id;
    }
    if current.selected_version != external.selected_version {
        current.selected_version = external.selected_version;
    }
    if current.servers != external.servers {
        current.servers = external.servers;
    }
    accounts_changed
}

#[cfg(test)]
mod tests {
    use rmcl_store::{
        DownloadSourcePreference, LauncherVisibility, StoredOfflineAccount, StoredServer,
        ThemePreference,
    };

    use super::*;

    #[test]
    fn external_config_merge_preserves_local_settings() {
        let mut current = LauncherConfig {
            auto_install_java: false,
            auto_check_updates: false,
            download_source: DownloadSourcePreference::Mirror,
            launcher_visibility: LauncherVisibility::HideWhileRunning,
            agent_provider: "anthropic".to_owned(),
            agent_model: "claude-test".to_owned(),
            agent_base_url: Some("https://agent.example".to_owned()),
            agent_proxy: Some("socks5://127.0.0.1:1080".to_owned()),
            ..LauncherConfig::default()
        };
        current.appearance.theme = ThemePreference::Dark;
        let external = LauncherConfig {
            selected_version: Some("external-instance".to_owned()),
            selected_account: Some("Alex".to_owned()),
            accounts: vec![StoredOfflineAccount {
                username: "Alex".to_owned(),
                uuid: "00000000-0000-0000-0000-000000000001".to_owned(),
            }],
            servers: vec![StoredServer {
                id: "external".to_owned(),
                name: "External".to_owned(),
                address: "play.example.com".to_owned(),
            }],
            ..LauncherConfig::default()
        };

        assert!(merge_external_config(&mut current, external.clone()));
        assert_eq!(
            current.selected_version.as_deref(),
            Some("external-instance")
        );
        assert_eq!(current.selected_account.as_deref(), Some("Alex"));
        assert_eq!(current.accounts, external.accounts);
        assert_eq!(current.servers, external.servers);
        assert!(!current.auto_install_java);
        assert!(!current.auto_check_updates);
        assert_eq!(current.download_source, DownloadSourcePreference::Mirror);
        assert_eq!(
            current.launcher_visibility,
            LauncherVisibility::HideWhileRunning
        );
        assert_eq!(current.appearance.theme, ThemePreference::Dark);
        assert_eq!(current.agent_provider, "anthropic");
        assert_eq!(current.agent_model, "claude-test");
        assert_eq!(
            current.agent_base_url.as_deref(),
            Some("https://agent.example")
        );
        assert_eq!(
            current.agent_proxy.as_deref(),
            Some("socks5://127.0.0.1:1080")
        );
        assert!(!merge_external_config(&mut current, external));
    }
}
