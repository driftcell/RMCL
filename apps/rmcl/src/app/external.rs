use rmcl_store::LauncherConfig;

/// Applies only external data that MCP can currently modify. Local service
/// settings and editor-owned values stay in the existing configuration.
pub fn merge_external_config(current: &mut LauncherConfig, external: LauncherConfig) -> bool {
    let accounts_changed = current.accounts != external.accounts
        || current.microsoft_accounts != external.microsoft_accounts
        || current.selected_account != external.selected_account
        || current.selected_account_kind != external.selected_account_kind;
    if accounts_changed {
        current.accounts = external.accounts;
        current.microsoft_accounts = external.microsoft_accounts;
        current.selected_account = external.selected_account;
        current.selected_account_kind = external.selected_account_kind;
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
            mcp_enabled: true,
            mcp_port: 40000,
            auto_install_java: false,
            auto_check_updates: false,
            download_source: DownloadSourcePreference::Mirror,
            launcher_visibility: LauncherVisibility::HideWhileRunning,
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
        assert!(current.mcp_enabled);
        assert_eq!(current.mcp_port, 40000);
        assert!(!current.auto_install_java);
        assert!(!current.auto_check_updates);
        assert_eq!(current.download_source, DownloadSourcePreference::Mirror);
        assert_eq!(
            current.launcher_visibility,
            LauncherVisibility::HideWhileRunning
        );
        assert_eq!(current.appearance.theme, ThemePreference::Dark);
        assert!(!merge_external_config(&mut current, external));
    }
}
