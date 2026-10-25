use std::time::Duration;

use reqwest::Client;
use rmcl_instance::LoaderKind;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::AppError;

const DEFAULT_USER_AGENT: &str = concat!(
    "RMCL/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/driftcell/RMCL)"
);

/// One loader version offered by a metadata source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderVersion {
    pub version: String,
    pub stable: bool,
}

#[derive(Debug, Clone)]
pub struct LoaderCatalogConfig {
    pub fabric_base: Url,
    pub quilt_base: Url,
    pub user_agent: String,
    pub timeout: Duration,
}

impl LoaderCatalogConfig {
    pub fn official() -> Result<Self, AppError> {
        Ok(Self {
            fabric_base: Url::parse("https://meta.fabricmc.net/")?,
            quilt_base: Url::parse("https://meta.quiltmc.org/")?,
            user_agent: DEFAULT_USER_AGENT.to_owned(),
            timeout: Duration::from_secs(30),
        })
    }
}

/// Metadata client for Fabric and Quilt loader versions and launcher profiles.
#[derive(Clone)]
pub struct LoaderCatalog {
    client: Client,
    fabric_base: Url,
    quilt_base: Url,
}

impl LoaderCatalog {
    pub fn new(config: LoaderCatalogConfig) -> Result<Self, AppError> {
        let client = Client::builder()
            .user_agent(config.user_agent)
            .timeout(config.timeout)
            .build()
            .map_err(|error| AppError::LoaderMetadata {
                message: error.to_string(),
            })?;
        Ok(Self {
            client,
            fabric_base: config.fabric_base,
            quilt_base: config.quilt_base,
        })
    }

    pub fn official() -> Result<Self, AppError> {
        Self::new(LoaderCatalogConfig::official()?)
    }

    pub async fn versions(
        &self,
        kind: LoaderKind,
        game_version: &str,
        cancellation: &CancellationToken,
    ) -> Result<Vec<LoaderVersion>, AppError> {
        let mut url = self.base(kind)?;
        {
            let mut segments =
                url.path_segments_mut()
                    .map_err(|_| AppError::LoaderMetadataInvalid {
                        message: "loader metadata base URL cannot be used as a path base"
                            .to_owned(),
                    })?;
            segments.pop_if_empty().push(game_version);
        }
        let entries: Vec<LoaderEntry> = self.get_json(url, cancellation).await?;
        Ok(entries
            .into_iter()
            .filter(|entry| !entry.loader.version.trim().is_empty())
            .map(|entry| LoaderVersion {
                version: entry.loader.version,
                stable: entry.loader.stable,
            })
            .collect())
    }

    pub async fn profile(
        &self,
        kind: LoaderKind,
        game_version: &str,
        loader_version: &str,
        cancellation: &CancellationToken,
    ) -> Result<String, AppError> {
        let mut url = self.base(kind)?;
        {
            let mut segments =
                url.path_segments_mut()
                    .map_err(|_| AppError::LoaderMetadataInvalid {
                        message: "loader metadata base URL cannot be used as a path base"
                            .to_owned(),
                    })?;
            segments
                .pop_if_empty()
                .push(game_version)
                .push(loader_version)
                .push("profile")
                .push("json");
        }
        let text = self.get_text(url, cancellation).await?;
        serde_json::from_str::<serde_json::Value>(&text).map_err(|error| {
            AppError::LoaderMetadataInvalid {
                message: error.to_string(),
            }
        })?;
        Ok(text)
    }

    fn base(&self, kind: LoaderKind) -> Result<Url, AppError> {
        let (base, prefix) = match kind {
            LoaderKind::Fabric => (&self.fabric_base, "v2/versions/loader/"),
            LoaderKind::Quilt => (&self.quilt_base, "v3/versions/loader/"),
            LoaderKind::Forge | LoaderKind::NeoForge => {
                return Err(AppError::InvalidRequest(format!(
                    "{} loader installation is not implemented yet",
                    kind.label()
                )));
            }
        };
        base.join(prefix).map_err(|error| AppError::LoaderMetadata {
            message: error.to_string(),
        })
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        url: Url,
        cancellation: &CancellationToken,
    ) -> Result<T, AppError> {
        let bytes = self.get_bytes(url, cancellation).await?;
        serde_json::from_slice(&bytes).map_err(|error| AppError::LoaderMetadataInvalid {
            message: error.to_string(),
        })
    }

    async fn get_text(
        &self,
        url: Url,
        cancellation: &CancellationToken,
    ) -> Result<String, AppError> {
        let bytes = self.get_bytes(url, cancellation).await?;
        String::from_utf8(bytes).map_err(|error| AppError::LoaderMetadataInvalid {
            message: error.to_string(),
        })
    }

    async fn get_bytes(
        &self,
        url: Url,
        cancellation: &CancellationToken,
    ) -> Result<Vec<u8>, AppError> {
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(AppError::Cancelled),
            response = self.client.get(url).send() => response.map_err(|error| AppError::LoaderMetadata {
                message: error.to_string(),
            })?,
        };
        let status = response.status();
        if !status.is_success() {
            let body = tokio::select! {
                _ = cancellation.cancelled() => return Err(AppError::Cancelled),
                body = response.text() => body.unwrap_or_default(),
            };
            return Err(AppError::LoaderMetadata {
                message: format!("HTTP {}: {}", status.as_u16(), truncate(&body, 300)),
            });
        }
        tokio::select! {
            _ = cancellation.cancelled() => Err(AppError::Cancelled),
            body = response.bytes() => body.map(|bytes| bytes.to_vec()).map_err(|error| AppError::LoaderMetadata {
                message: error.to_string(),
            }),
        }
    }
}

fn truncate(value: &str, limit: usize) -> String {
    let mut truncated = value.chars().take(limit).collect::<String>();
    if value.chars().count() > limit {
        truncated.push('…');
    }
    truncated
}

#[derive(Debug, Deserialize)]
struct LoaderEntry {
    loader: LoaderInfo,
}

#[derive(Debug, Deserialize)]
struct LoaderInfo {
    version: String,
    #[serde(default)]
    stable: bool,
}

#[cfg(test)]
mod tests {
    use axum::{Json, Router, http::StatusCode, routing::get};
    use serde_json::json;

    use super::*;

    async fn server(router: Router) -> Url {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        base
    }

    fn catalog(fabric_base: Url, quilt_base: Url) -> LoaderCatalog {
        LoaderCatalog::new(LoaderCatalogConfig {
            fabric_base,
            quilt_base,
            user_agent: "RMCL-test".to_owned(),
            timeout: Duration::from_secs(5),
        })
        .unwrap()
    }

    #[tokio::test]
    async fn fabric_versions_and_quilt_profile_are_parsed() {
        let fabric_base = server(Router::new().route(
            "/v2/versions/loader/1.21.1",
            get(|| async {
                Json(json!([
                    {"loader": {"version": "0.16.10", "stable": true}},
                    {"loader": {"version": "0.16.11", "stable": false}}
                ]))
            }),
        ))
        .await;
        let quilt_base = server(Router::new().route(
            "/v3/versions/loader/1.21.1/0.20.0-beta.9/profile/json",
            get(|| async {
                Json(json!({
                    "id": "quilt-loader-0.20.0-beta.9-1.21.1",
                    "inheritsFrom": "1.21.1",
                    "mainClass": "example.Main",
                    "libraries": []
                }))
            }),
        ))
        .await;
        let catalog = catalog(fabric_base, quilt_base);

        let versions = catalog
            .versions(LoaderKind::Fabric, "1.21.1", &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, "0.16.10");
        assert!(versions[0].stable);

        let profile = catalog
            .profile(
                LoaderKind::Quilt,
                "1.21.1",
                "0.20.0-beta.9",
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(profile.contains("quilt-loader-0.20.0-beta.9-1.21.1"));
    }

    #[tokio::test]
    async fn loader_metadata_errors_are_structured() {
        let fabric_base = server(Router::new().route(
            "/v2/versions/loader/1.21.1",
            get(|| async { (StatusCode::NOT_FOUND, "missing") }),
        ))
        .await;
        let catalog = catalog(fabric_base.clone(), fabric_base);

        let error = catalog
            .versions(LoaderKind::Fabric, "1.21.1", &CancellationToken::new())
            .await
            .unwrap_err();

        assert!(matches!(error, AppError::LoaderMetadata { .. }));
    }
}
