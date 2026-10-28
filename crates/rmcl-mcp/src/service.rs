use std::sync::{Arc, Mutex, MutexGuard};

use rmcl_app::AppContext;
pub use rmcl_store::DEFAULT_MCP_PORT;
use tokio_util::sync::CancellationToken;

use crate::{HttpServerConfig, HttpServerHandle};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpHttpStatus {
    pub enabled: bool,
    pub port: u16,
    pub running: bool,
    pub error: Option<String>,
}

impl Default for McpHttpStatus {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_MCP_PORT,
            running: false,
            error: None,
        }
    }
}

/// Owns the optional GUI-hosted HTTP server independently of any View.
pub struct McpHttpService {
    context: Arc<AppContext>,
    lifecycle: tokio::sync::Mutex<()>,
    server: tokio::sync::Mutex<Option<HttpServerHandle>>,
    status: Mutex<McpHttpStatus>,
}

impl McpHttpService {
    pub fn new(context: Arc<AppContext>) -> Self {
        Self {
            context,
            lifecycle: tokio::sync::Mutex::new(()),
            server: tokio::sync::Mutex::new(None),
            status: Mutex::new(McpHttpStatus::default()),
        }
    }

    pub fn status(&self) -> McpHttpStatus {
        self.status_lock().clone()
    }

    /// Stops the previous server and applies a new loopback, read-only configuration.
    /// Failures are returned as status so a bad port never prevents GUI startup.
    pub async fn reconcile(&self, enabled: bool, port: u16) -> McpHttpStatus {
        let _lifecycle = self.lifecycle.lock().await;
        let stop_error = self.stop_current().await;
        if !enabled {
            return self.update_status(enabled, port, false, stop_error);
        }
        if let Some(error) = stop_error {
            return self.update_status(enabled, port, false, Some(error));
        }
        if port == 0 {
            return self.update_status(
                enabled,
                port,
                false,
                Some("MCP port must be between 1 and 65535".to_owned()),
            );
        }
        let config = HttpServerConfig::new(port);
        match HttpServerHandle::start(self.context.clone(), config, CancellationToken::new()).await
        {
            Ok(server) => {
                *self.server.lock().await = Some(server);
                self.update_status(enabled, port, true, None)
            }
            Err(error) => self.update_status(enabled, port, false, Some(error.to_string())),
        }
    }

    async fn stop_current(&self) -> Option<String> {
        let server = self.server.lock().await.take();
        match server {
            Some(server) => server.stop().await.err().map(|error| error.to_string()),
            None => None,
        }
    }

    fn update_status(
        &self,
        enabled: bool,
        port: u16,
        running: bool,
        error: Option<String>,
    ) -> McpHttpStatus {
        let status = McpHttpStatus {
            enabled,
            port,
            running,
            error,
        };
        *self.status_lock() = status.clone();
        status
    }

    fn status_lock(&self) -> MutexGuard<'_, McpHttpStatus> {
        self.status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use rmcl_store::{ConfigStore, LauncherConfig};
    use tempfile::TempDir;

    use super::*;

    fn context(temporary: &TempDir) -> Arc<AppContext> {
        let config = LauncherConfig {
            minecraft_root: temporary.path().join("minecraft"),
            java_store: temporary.path().join("java"),
            auto_install_java: false,
            ..LauncherConfig::default()
        };
        AppContext::new(
            &config,
            ConfigStore::new(temporary.path().join("config.toml")),
        )
        .unwrap()
    }

    fn unused_port() -> u16 {
        std::net::TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    #[test]
    fn gui_service_reports_bind_failures_and_stops_cleanly() {
        let temporary = TempDir::new().unwrap();
        let context = context(&temporary);
        let port = unused_port();
        let service = McpHttpService::new(context.clone());
        context.runtime.block_on(async {
            let started = service.reconcile(true, port).await;
            assert!(started.running);
            assert_eq!(started.error, None);

            let conflicting = McpHttpService::new(context.clone());
            let blocked = conflicting.reconcile(true, port).await;
            assert!(!blocked.running);
            assert!(blocked.error.is_some());
            drop(conflicting);

            let stopped = service.reconcile(false, port).await;
            assert!(!stopped.running);
            assert_eq!(stopped.error, None);
            let rebound = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
            drop(rebound);

            let restarted = service.reconcile(true, port).await;
            assert!(restarted.running);
            let invalid = service.reconcile(true, 0).await;
            assert!(!invalid.running);
            assert!(invalid.error.is_some());

            let stopped = service.reconcile(false, port).await;
            assert!(!stopped.running);
            assert_eq!(stopped.error, None);
        });
        drop(service);
        match Arc::try_unwrap(context) {
            Ok(context) => context.runtime.shutdown_background(),
            Err(_) => panic!("app context should have no remaining owners"),
        }
    }
}
