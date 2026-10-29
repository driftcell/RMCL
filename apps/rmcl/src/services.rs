use std::{
    ops::Deref,
    path::{Path, PathBuf},
    sync::Arc,
};

use clap::{ArgGroup, Args, Parser, Subcommand};
use rmcl_agent::{AGENT_KEYRING_SERVICE, AgentKeyStore, AgentService};
use rmcl_app::AppContext;
use rmcl_auth::{MicrosoftAccount, OfflineAccount};
use rmcl_mcp::{McpHttpService, McpHttpStatus};
use rmcl_platform::KeyringSecretStore;
use rmcl_store::{
    CONFIG_SCHEMA_VERSION, ConfigStore, LauncherConfig, StoredAccountKind, StoredMicrosoftAccount,
    StoredOfflineAccount, default_java_store, default_minecraft_root,
};
use uuid::Uuid;

pub(crate) struct Services {
    pub(crate) context: Arc<AppContext>,
    pub(crate) mcp: McpHttpService,
    pub(crate) agent: Arc<AgentService>,
}

impl Services {
    pub(crate) fn new(
        config: &LauncherConfig,
        config_store: ConfigStore,
    ) -> Result<Arc<Self>, rmcl_app::AppError> {
        let context = AppContext::new(config, config_store)?;
        let mcp = McpHttpService::new(context.clone());
        let agent_secrets = Arc::new(KeyringSecretStore::new(AGENT_KEYRING_SERVICE)?);
        let agent = Arc::new(AgentService::new(
            context.clone(),
            AgentKeyStore::new(agent_secrets),
        ));
        Ok(Arc::new(Self {
            context,
            mcp,
            agent,
        }))
    }

    pub(crate) fn mcp_status(&self) -> McpHttpStatus {
        self.mcp.status()
    }

    pub(crate) async fn reconcile_mcp(&self, enabled: bool, port: u16) -> McpHttpStatus {
        self.mcp.reconcile(enabled, port).await
    }
}

impl Deref for Services {
    type Target = AppContext;

    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

#[derive(Parser)]
#[command(name = "rmcl", about = "A cross-platform Minecraft launcher")]
pub(crate) struct LauncherArgs {
    #[arg(long, value_name = "PLAN", hide = true)]
    pub(crate) apply_update: Option<PathBuf>,
    /// Launcher configuration (default: platform configuration directory).
    #[arg(long, value_name = "PATH", global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    pub(crate) command: Option<LauncherCommand>,
    /// Override Minecraft repository for this run.
    #[arg(long, value_name = "PATH")]
    minecraft_root: Option<PathBuf>,
    /// Override managed Java store for this run.
    #[arg(long, value_name = "PATH")]
    java_store: Option<PathBuf>,
    /// Override the initially selected version.
    #[arg(long, value_name = "ID")]
    version: Option<String>,
    /// Add or select an offline player.
    #[arg(long, value_name = "NAME")]
    player: Option<String>,
}

#[derive(Clone, Subcommand)]
pub(crate) enum LauncherCommand {
    /// Expose RMCL instances, content, and tasks over MCP.
    Mcp(McpArgs),
}

#[derive(Clone, Args)]
#[command(group = ArgGroup::new("transport").required(true).args(["stdio", "http"]))]
pub(crate) struct McpArgs {
    /// Serve MCP over standard input and output.
    #[arg(long)]
    pub(crate) stdio: bool,
    /// Serve MCP over Streamable HTTP on loopback.
    #[arg(long)]
    pub(crate) http: bool,
    /// Loopback TCP port for the HTTP transport.
    #[arg(long, default_value_t = 39271, requires = "http")]
    pub(crate) port: u16,
    /// HTTP access level: read_only or read_write.
    #[arg(long, default_value = "read_only", value_parser = ["read_only", "read_write"], requires = "http")]
    pub(crate) access: String,
    /// Bearer token for HTTP clients; RMCL_MCP_TOKEN is preferred for secrets.
    #[arg(long, value_name = "TOKEN", requires = "http")]
    pub(crate) token: Option<String>,
    /// Allowed browser Origin; repeat for multiple origins.
    #[arg(long = "origin", value_name = "ORIGIN", requires = "http")]
    pub(crate) allowed_origins: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Bootstrap {
    pub config: LauncherConfig,
    pub config_store: ConfigStore,
    pub config_warning: Option<String>,
    /// Whether startup normalization may be written back to the configuration.
    pub save_on_start: bool,
}

impl Bootstrap {
    pub(crate) fn from_arguments(arguments: LauncherArgs) -> Result<Self, String> {
        let LauncherArgs {
            apply_update: _,
            config,
            command: _,
            minecraft_root,
            java_store,
            version,
            player,
        } = arguments;
        let config_store = match config {
            Some(path) => ConfigStore::new(path),
            None => ConfigStore::discover().map_err(|error| error.to_string())?,
        };
        let has_overrides = minecraft_root.is_some()
            || java_store.is_some()
            || version.is_some()
            || player.is_some();
        let config_existed = config_store.path().exists();
        let (mut config, config_warning) = match config_store.load() {
            Ok(config) => (config, None),
            Err(error) => (LauncherConfig::default(), Some(error.to_string())),
        };
        config.schema_version = CONFIG_SCHEMA_VERSION;

        if !config_existed && config.accounts.is_empty() && config.microsoft_accounts.is_empty() {
            select_offline_account(
                &mut config,
                OfflineAccount::new("Steve").map_err(|e| e.to_string())?,
            );
        }
        if let Some(root) = minecraft_root {
            config.minecraft_root = root;
        }
        if let Some(store) = java_store {
            config.java_store = store;
        }
        if let Some(version) = version {
            config.selected_version = Some(version);
        }
        if let Some(player) = player {
            select_offline_account(
                &mut config,
                OfflineAccount::new(player).map_err(|e| e.to_string())?,
            );
        }
        ensure_selected_account(&mut config);
        if config.minecraft_root.as_os_str().is_empty() {
            config.minecraft_root = default_minecraft_root().map_err(|error| error.to_string())?;
        }
        if config.java_store.as_os_str().is_empty() {
            config.java_store = default_java_store().map_err(|error| error.to_string())?;
        }

        Ok(Self {
            config,
            config_store,
            config_warning: config_warning.clone(),
            save_on_start: should_save_on_start(
                config_existed,
                has_overrides,
                config_warning.as_deref(),
            ),
        })
    }
}

pub fn stored_account(account: &OfflineAccount) -> StoredOfflineAccount {
    StoredOfflineAccount {
        username: account.username().to_owned(),
        uuid: account.uuid().to_string(),
    }
}

pub fn load_account(stored: &StoredOfflineAccount) -> Option<OfflineAccount> {
    match Uuid::parse_str(&stored.uuid) {
        Ok(uuid) => OfflineAccount::with_uuid(stored.username.clone(), uuid).ok(),
        // A damaged UUID must not make an otherwise usable offline profile disappear.
        Err(_) => OfflineAccount::new(stored.username.clone()).ok(),
    }
}

pub fn stored_microsoft_account(account: &MicrosoftAccount) -> StoredMicrosoftAccount {
    StoredMicrosoftAccount {
        username: account.username().to_owned(),
        uuid: account.uuid().to_string(),
        skin_url: account.skin_url().map(str::to_owned),
    }
}

pub fn load_microsoft_account(stored: &StoredMicrosoftAccount) -> Option<MicrosoftAccount> {
    Uuid::parse_str(&stored.uuid).ok().and_then(|uuid| {
        MicrosoftAccount::new(stored.username.clone(), uuid)
            .ok()
            .map(|account| account.with_skin_url(stored.skin_url.clone()))
    })
}

fn select_offline_account(config: &mut LauncherConfig, account: OfflineAccount) {
    config
        .accounts
        .retain(|stored| stored.username != account.username());
    config.accounts.push(stored_account(&account));
    config.selected_account = Some(account.username().to_owned());
    config.selected_account_kind = StoredAccountKind::Offline;
}

fn ensure_selected_account(config: &mut LauncherConfig) {
    let exists = match config.selected_account_kind {
        StoredAccountKind::Offline => config.selected_account.as_ref().is_some_and(|selected| {
            config
                .accounts
                .iter()
                .any(|account| account.username == *selected)
        }),
        StoredAccountKind::Microsoft => config.selected_account.as_ref().is_some_and(|selected| {
            config
                .microsoft_accounts
                .iter()
                .any(|account| account.username == *selected)
        }),
    };
    if exists {
        return;
    }
    if let Some(account) = config.accounts.first() {
        config.selected_account = Some(account.username.clone());
        config.selected_account_kind = StoredAccountKind::Offline;
    } else if let Some(account) = config.microsoft_accounts.first() {
        config.selected_account = Some(account.username.clone());
        config.selected_account_kind = StoredAccountKind::Microsoft;
    } else {
        config.selected_account = None;
        config.selected_account_kind = StoredAccountKind::Offline;
    }
}

fn should_save_on_start(config_existed: bool, has_overrides: bool, warning: Option<&str>) -> bool {
    warning.is_none() && (!config_existed || !has_overrides)
}

/// Opens a local file or directory with the platform's default handler.
pub fn open_path(path: &Path) -> std::io::Result<()> {
    open::that(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_help_documents_all_overrides() {
        use clap::error::ErrorKind;

        let error = match LauncherArgs::try_parse_from(["rmcl", "--help"]) {
            Err(error) => error,
            Ok(_) => panic!("--help should return a clap display error"),
        };
        assert_eq!(error.kind(), ErrorKind::DisplayHelp);
        let help = error.to_string();
        for option in [
            "--config",
            "--minecraft-root",
            "--java-store",
            "--version",
            "--player",
        ] {
            assert!(help.contains(option), "help should contain {option}");
        }
        assert!(!help.contains("--apply-update"));
        assert!(help.contains("mcp"));
    }

    #[test]
    fn cli_parses_mcp_transports_from_the_same_binary() {
        let arguments =
            LauncherArgs::try_parse_from(["rmcl", "mcp", "--stdio", "--config", "mcp-config.toml"])
                .unwrap();
        assert_eq!(
            arguments.config.as_deref(),
            Some(Path::new("mcp-config.toml"))
        );
        let Some(LauncherCommand::Mcp(mcp)) = arguments.command else {
            panic!("expected MCP subcommand");
        };
        assert!(mcp.stdio);
        assert!(!mcp.http);
        assert_eq!(mcp.access, "read_only");
        assert!(mcp.token.is_none());
        assert!(mcp.allowed_origins.is_empty());

        let arguments = LauncherArgs::try_parse_from([
            "rmcl",
            "mcp",
            "--http",
            "--port",
            "40000",
            "--access",
            "read_write",
            "--token",
            "test-token",
            "--origin",
            "https://agent.example",
            "--origin",
            "https://other.example",
        ])
        .unwrap();
        let Some(LauncherCommand::Mcp(mcp)) = arguments.command else {
            panic!("expected MCP subcommand");
        };
        assert!(mcp.http);
        assert_eq!(mcp.port, 40000);
        assert_eq!(mcp.access, "read_write");
        assert_eq!(mcp.token.as_deref(), Some("test-token"));
        assert_eq!(
            mcp.allowed_origins,
            ["https://agent.example", "https://other.example"]
        );

        assert!(LauncherArgs::try_parse_from(["rmcl", "mcp"]).is_err());
        assert!(LauncherArgs::try_parse_from(["rmcl", "mcp", "--stdio", "--http"]).is_err());
    }

    #[test]
    fn cli_parses_all_overrides() {
        let arguments = LauncherArgs::try_parse_from([
            "rmcl",
            "--config",
            "config.toml",
            "--minecraft-root",
            "minecraft",
            "--java-store",
            "java",
            "--version",
            "1.21.1",
            "--player",
            "Steve",
        ])
        .unwrap();

        assert_eq!(arguments.config.as_deref(), Some(Path::new("config.toml")));
        assert_eq!(
            arguments.minecraft_root.as_deref(),
            Some(Path::new("minecraft"))
        );
        assert_eq!(arguments.java_store.as_deref(), Some(Path::new("java")));
        assert_eq!(arguments.version.as_deref(), Some("1.21.1"));
        assert_eq!(arguments.player.as_deref(), Some("Steve"));
    }

    #[test]
    fn cli_parses_hidden_update_helper_mode() {
        let arguments =
            LauncherArgs::try_parse_from(["rmcl", "--apply-update", "update-plan.json"]).unwrap();

        assert_eq!(
            arguments.apply_update.as_deref(),
            Some(Path::new("update-plan.json"))
        );
    }

    #[test]
    fn cli_reports_missing_values_with_help() {
        let error = match LauncherArgs::try_parse_from(["rmcl", "--config"]) {
            Err(error) => error,
            Ok(_) => panic!("missing --config value should be rejected"),
        };
        let message = error.to_string();
        assert!(message.contains("a value is required"));
        assert!(message.contains("try '--help'"));
    }

    #[test]
    fn startup_writes_do_not_persist_command_line_overrides() {
        assert!(should_save_on_start(false, true, None));
        assert!(!should_save_on_start(true, true, None));
        assert!(should_save_on_start(true, false, None));
        assert!(!should_save_on_start(false, false, Some("broken config")));
    }

    #[test]
    fn stored_accounts_round_trip_the_derived_uuid() {
        let account = OfflineAccount::new("Steve").unwrap();
        let stored = stored_account(&account);
        let loaded = load_account(&stored).unwrap();

        assert_eq!(loaded.username(), account.username());
        assert_eq!(loaded.uuid(), account.uuid());
    }

    #[test]
    fn damaged_uuid_falls_back_to_the_offline_identity() {
        let stored = StoredOfflineAccount {
            username: "Alex".to_owned(),
            uuid: "not-a-uuid".to_owned(),
        };

        assert_eq!(
            load_account(&stored).unwrap().uuid(),
            OfflineAccount::new("Alex").unwrap().uuid()
        );
    }

    #[test]
    fn microsoft_accounts_round_trip_their_profile_uuid() {
        let uuid = Uuid::parse_str("986dec87-b7ec-47ff-89ff-033fdb95c4b5").unwrap();
        let account = MicrosoftAccount::new("HowDoesAuthWork", uuid).unwrap();
        let stored = stored_microsoft_account(&account);
        let loaded = load_microsoft_account(&stored).unwrap();

        assert_eq!(loaded.username(), account.username());
        assert_eq!(loaded.uuid(), account.uuid());
    }

    #[test]
    fn damaged_microsoft_uuid_disables_the_account() {
        let stored = StoredMicrosoftAccount {
            username: "HowDoesAuthWork".to_owned(),
            uuid: "not-a-uuid".to_owned(),
            skin_url: None,
        };

        assert!(load_microsoft_account(&stored).is_none());
    }

    #[test]
    fn missing_selection_falls_back_to_an_available_account() {
        let mut config = LauncherConfig::default();
        config.microsoft_accounts.push(StoredMicrosoftAccount {
            username: "HowDoesAuthWork".to_owned(),
            uuid: "986dec87-b7ec-47ff-89ff-033fdb95c4b5".to_owned(),
            skin_url: None,
        });

        ensure_selected_account(&mut config);

        assert_eq!(config.selected_account.as_deref(), Some("HowDoesAuthWork"));
        assert_eq!(config.selected_account_kind, StoredAccountKind::Microsoft);
    }
}
