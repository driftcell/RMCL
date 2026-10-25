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
    /// Maven/installer identifier; equal to `version` for Fabric and Quilt.
    pub installer_version: String,
    pub stable: bool,
}

#[derive(Debug, Clone)]
pub struct LoaderCatalogConfig {
    pub fabric_base: Url,
    pub quilt_base: Url,
    pub forge_metadata_base: Url,
    pub forge_maven_base: Url,
    pub neoforge_api_base: Url,
    pub neoforge_maven_base: Url,
    pub user_agent: String,
    pub timeout: Duration,
}

impl LoaderCatalogConfig {
    pub fn official() -> Result<Self, AppError> {
        Ok(Self {
            fabric_base: Url::parse("https://meta.fabricmc.net/")?,
            quilt_base: Url::parse("https://meta.quiltmc.org/")?,
            forge_metadata_base: Url::parse(
                "https://files.minecraftforge.net/net/minecraftforge/forge/",
            )?,
            forge_maven_base: Url::parse("https://maven.minecraftforge.net/")?,
            neoforge_api_base: Url::parse(
                "https://maven.neoforged.net/api/maven/versions/releases/",
            )?,
            neoforge_maven_base: Url::parse("https://maven.neoforged.net/releases/")?,
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
    forge_metadata_base: Url,
    forge_maven_base: Url,
    neoforge_api_base: Url,
    neoforge_maven_base: Url,
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
            forge_metadata_base: config.forge_metadata_base,
            forge_maven_base: config.forge_maven_base,
            neoforge_api_base: config.neoforge_api_base,
            neoforge_maven_base: config.neoforge_maven_base,
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
        match kind {
            LoaderKind::Fabric | LoaderKind::Quilt => {
                let mut url = self.meta_base(kind)?;
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
                        version: entry.loader.version.clone(),
                        installer_version: entry.loader.version,
                        stable: entry.loader.stable,
                    })
                    .collect())
            }
            LoaderKind::Forge => self.forge_versions(game_version, cancellation).await,
            LoaderKind::NeoForge => self.neoforge_versions(game_version, cancellation).await,
        }
    }

    pub async fn profile(
        &self,
        kind: LoaderKind,
        game_version: &str,
        loader_version: &str,
        cancellation: &CancellationToken,
    ) -> Result<String, AppError> {
        if matches!(kind, LoaderKind::Forge | LoaderKind::NeoForge) {
            return Err(AppError::InvalidRequest(
                "Forge/NeoForge use installer JARs instead of launcher profiles".to_owned(),
            ));
        }
        let mut url = self.meta_base(kind)?;
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

    pub fn installer_url(
        &self,
        kind: LoaderKind,
        game_version: &str,
        installer_version: &str,
    ) -> Result<Url, AppError> {
        let (base, segments) = match kind {
            LoaderKind::Forge => (
                &self.forge_maven_base,
                vec![
                    "net".to_owned(),
                    "minecraftforge".to_owned(),
                    "forge".to_owned(),
                    installer_version.to_owned(),
                    format!("forge-{installer_version}-installer.jar"),
                ],
            ),
            LoaderKind::NeoForge => {
                let (group, artifact) = if game_version == "1.20.1" {
                    ("forge", "forge")
                } else {
                    ("neoforge", "neoforge")
                };
                (
                    &self.neoforge_maven_base,
                    vec![
                        "net".to_owned(),
                        "neoforged".to_owned(),
                        group.to_owned(),
                        installer_version.to_owned(),
                        format!("{artifact}-{installer_version}-installer.jar"),
                    ],
                )
            }
            LoaderKind::Fabric | LoaderKind::Quilt => {
                return Err(AppError::InvalidRequest(
                    "Fabric/Quilt do not use installer JARs".to_owned(),
                ));
            }
        };
        let mut url = base.join("").map_err(|error| AppError::LoaderMetadata {
            message: error.to_string(),
        })?;
        {
            let mut path =
                url.path_segments_mut()
                    .map_err(|_| AppError::LoaderMetadataInvalid {
                        message: "loader maven base URL cannot be used as a path base".to_owned(),
                    })?;
            path.pop_if_empty();
            for segment in segments {
                path.push(&segment);
            }
        }
        Ok(url)
    }

    async fn forge_versions(
        &self,
        game_version: &str,
        cancellation: &CancellationToken,
    ) -> Result<Vec<LoaderVersion>, AppError> {
        if !forge_supports_processors(game_version) {
            return Err(AppError::InvalidRequest(format!(
                "Forge for Minecraft `{game_version}` uses the legacy installer format, which is not supported yet"
            )));
        }
        let url = self
            .forge_metadata_base
            .join("maven-metadata.json")
            .map_err(|error| AppError::LoaderMetadata {
                message: error.to_string(),
            })?;
        let metadata: std::collections::BTreeMap<String, Vec<String>> =
            self.get_json(url, cancellation).await?;
        let prefix = format!("{game_version}-");
        let mut versions = metadata
            .get(game_version)
            .into_iter()
            .flatten()
            .filter_map(|installer_version| {
                let display = installer_version.strip_prefix(&prefix)?;
                Some(LoaderVersion {
                    version: display.to_owned(),
                    installer_version: installer_version.clone(),
                    stable: is_stable(display),
                })
            })
            .collect::<Vec<_>>();
        sort_versions(&mut versions);
        Ok(versions)
    }

    async fn neoforge_versions(
        &self,
        game_version: &str,
        cancellation: &CancellationToken,
    ) -> Result<Vec<LoaderVersion>, AppError> {
        let old = game_version == "1.20.1";
        let path = if old {
            "net/neoforged/forge"
        } else {
            "net/neoforged/neoforge"
        };
        let url = self
            .neoforge_api_base
            .join(path)
            .map_err(|error| AppError::LoaderMetadata {
                message: error.to_string(),
            })?;
        let response: NeoForgeVersions = self.get_json(url, cancellation).await?;
        let mut versions = response
            .versions
            .into_iter()
            .filter_map(|installer_version| {
                let display = if old {
                    normalize_neoforge_old_version(&installer_version)
                } else {
                    if neoforge_mc_version(&installer_version)?.as_str() != game_version {
                        return None;
                    }
                    installer_version.clone()
                };
                Some(LoaderVersion {
                    version: display.clone(),
                    installer_version,
                    stable: is_stable(&display),
                })
            })
            .collect::<Vec<_>>();
        sort_versions(&mut versions);
        Ok(versions)
    }

    fn meta_base(&self, kind: LoaderKind) -> Result<Url, AppError> {
        let (base, prefix) = match kind {
            LoaderKind::Fabric => (&self.fabric_base, "v2/versions/loader/"),
            LoaderKind::Quilt => (&self.quilt_base, "v3/versions/loader/"),
            LoaderKind::Forge | LoaderKind::NeoForge => {
                return Err(AppError::InvalidRequest(
                    "Forge/NeoForge do not expose launcher profiles".to_owned(),
                ));
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

fn forge_supports_processors(game_version: &str) -> bool {
    // Forge switched to the spec/processor installer in 1.13. New-style
    // Minecraft versions (for example 26.3) all use it as well.
    let Some(rest) = game_version.strip_prefix("1.") else {
        return true;
    };
    rest.split('.')
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .is_none_or(|minor| minor >= 13)
}

/// Extracts the Minecraft version encoded in a NeoForge Maven version.
///
/// NeoForge uses `21.1.80` for Minecraft 1.21.1 and `26.3.1` for Minecraft
/// 26.3. Snapshot builds start with `0.` and encode the game version between
/// the first two dots, mirroring the official Maven artifact naming.
fn neoforge_mc_version(version: &str) -> Option<String> {
    let first = version.find('.')?;
    let second = version[first + 1..]
        .find('.')
        .map(|index| index + first + 1)?;
    let major = version[..first].parse::<u32>().ok()?;
    if major == 0 {
        return Some(version[first + 1..second].to_owned());
    }
    if major >= 26 {
        let third = version[second + 1..]
            .find('.')
            .map(|index| index + second + 1)?;
        let patch = version[second + 1..third].parse::<u32>().ok()?;
        let base = if patch == 0 {
            &version[..second]
        } else {
            &version[..third]
        };
        return Some(match version.find('+') {
            Some(index) => format!("{base}-{}", &version[index + 1..]),
            None => base.to_owned(),
        });
    }
    let minor = version[first + 1..second].parse::<u32>().ok()?;
    let base = if minor == 0 {
        &version[..first]
    } else {
        &version[..second]
    };
    Some(format!("1.{base}"))
}

fn normalize_neoforge_old_version(version: &str) -> String {
    version
        .strip_prefix("1.20.1-forge-")
        .or_else(|| version.strip_prefix("1.20.1-"))
        .unwrap_or(version)
        .to_owned()
}

fn is_stable(version: &str) -> bool {
    let lower = version.to_ascii_lowercase();
    !lower.contains("beta")
        && !lower.contains("alpha")
        && !lower.contains("pre")
        && !lower.contains("snapshot")
}

fn sort_versions(versions: &mut [LoaderVersion]) {
    versions.sort_by(|left, right| {
        numeric_key(&right.version)
            .cmp(&numeric_key(&left.version))
            .then_with(|| right.stable.cmp(&left.stable))
            .then_with(|| right.version.cmp(&left.version))
    });
}

fn numeric_key(value: &str) -> Vec<u64> {
    value
        .split(|character: char| !character.is_ascii_digit())
        .filter_map(|part| part.parse::<u64>().ok())
        .collect()
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

#[derive(Debug, Deserialize)]
struct NeoForgeVersions {
    #[serde(default)]
    versions: Vec<String>,
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
            forge_metadata_base: Url::parse("https://example.invalid/").unwrap(),
            forge_maven_base: Url::parse("https://example.invalid/").unwrap(),
            neoforge_api_base: Url::parse("https://example.invalid/").unwrap(),
            neoforge_maven_base: Url::parse("https://example.invalid/").unwrap(),
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
        assert_eq!(versions[0].installer_version, "0.16.10");
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
    async fn forge_and_neoforge_versions_and_installer_urls_work() {
        let base = server(
            Router::new()
                .route(
                    "/maven-metadata.json",
                    get(|| async {
                        Json(json!({
                            "1.21.1": [
                                "1.21.1-52.1.20",
                                "1.21.1-52.1.9",
                                "1.21.1-52.1.10"
                            ],
                            "26.3": ["26.3-61.0.1"],
                            "1.12.2": ["1.12.2-14.23.5.2860"]
                        }))
                    }),
                )
                .route(
                    "/net/neoforged/neoforge",
                    get(|| async {
                        Json(json!({
                            "versions": [
                                "26.3.0.2",
                                "26.3.0.1",
                                "21.1.80",
                                "21.1.9",
                                "21.1.80-beta",
                                "20.2.3-beta",
                                "0.25w14craftmine.3-beta"
                            ]
                        }))
                    }),
                )
                .route(
                    "/net/neoforged/forge",
                    get(|| async {
                        Json(json!({
                            "versions": [
                                "1.20.1-47.1.106",
                                "1.20.1-forge-47.1.100"
                            ]
                        }))
                    }),
                ),
        )
        .await;
        let catalog = LoaderCatalog::new(LoaderCatalogConfig {
            fabric_base: base.clone(),
            quilt_base: base.clone(),
            forge_metadata_base: base.clone(),
            forge_maven_base: base.clone(),
            neoforge_api_base: base.clone(),
            neoforge_maven_base: base.clone(),
            user_agent: "RMCL-test".to_owned(),
            timeout: Duration::from_secs(5),
        })
        .unwrap();

        let forge = catalog
            .versions(LoaderKind::Forge, "1.21.1", &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(forge[0].version, "52.1.20");
        assert_eq!(forge[0].installer_version, "1.21.1-52.1.20");
        let forge_url = catalog
            .installer_url(LoaderKind::Forge, "1.21.1", &forge[0].installer_version)
            .unwrap();
        assert!(forge_url.as_str().ends_with(
            "/net/minecraftforge/forge/1.21.1-52.1.20/forge-1.21.1-52.1.20-installer.jar"
        ));

        let legacy = catalog
            .versions(LoaderKind::Forge, "1.12.2", &CancellationToken::new())
            .await
            .unwrap_err();
        assert!(matches!(legacy, AppError::InvalidRequest(_)));

        let modern_forge = catalog
            .versions(LoaderKind::Forge, "26.3", &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(modern_forge[0].installer_version, "26.3-61.0.1");

        let neo = catalog
            .versions(LoaderKind::NeoForge, "1.21.1", &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(neo[0].version, "21.1.80");
        assert!(neo[0].stable);
        assert_eq!(neo[1].version, "21.1.80-beta");
        assert_eq!(neo[2].version, "21.1.9");
        let neo_url = catalog
            .installer_url(LoaderKind::NeoForge, "1.21.1", &neo[2].installer_version)
            .unwrap();
        assert!(
            neo_url
                .as_str()
                .ends_with("/net/neoforged/neoforge/21.1.9/neoforge-21.1.9-installer.jar")
        );

        let neo_263 = catalog
            .versions(LoaderKind::NeoForge, "26.3", &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(neo_263.len(), 2);
        assert_eq!(neo_263[0].version, "26.3.0.2");
        assert_eq!(neo_263[0].installer_version, "26.3.0.2");

        let neo_old = catalog
            .versions(LoaderKind::NeoForge, "1.20.1", &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(neo_old[0].version, "47.1.106");
        assert_eq!(neo_old[1].version, "47.1.100");
        let neo_old_url = catalog
            .installer_url(
                LoaderKind::NeoForge,
                "1.20.1",
                &neo_old[0].installer_version,
            )
            .unwrap();
        assert!(
            neo_old_url.as_str().ends_with(
                "/net/neoforged/forge/1.20.1-47.1.106/forge-1.20.1-47.1.106-installer.jar"
            )
        );
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
