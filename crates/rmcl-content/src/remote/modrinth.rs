use std::{collections::HashMap, time::Duration};

use reqwest::Client;
use serde::{Deserialize, de::DeserializeOwned};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::ContentKind;

use super::{
    ContentSource, RemoteContentError, RemoteDependency, RemoteDependencyKind, RemoteFile,
    RemoteProject, RemoteSearchPage, RemoteSearchQuery, RemoteVersion, model::parse_sha1,
};

const MODRINTH: ContentSource = ContentSource::Modrinth;
const DEFAULT_USER_AGENT: &str = concat!(
    "RMCL/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/driftcell/RMCL)"
);

/// Configuration for the Modrinth metadata client.
#[derive(Debug, Clone)]
pub struct ModrinthConfig {
    /// Base API URL. It must end with `/` so relative endpoints join correctly.
    pub base_url: Url,
    pub user_agent: String,
    pub timeout: Duration,
}

impl ModrinthConfig {
    pub fn new(base_url: Url) -> Self {
        Self {
            base_url,
            user_agent: DEFAULT_USER_AGENT.to_owned(),
            timeout: Duration::from_secs(30),
        }
    }

    pub fn official() -> Result<Self, RemoteContentError> {
        let base_url = Url::parse("https://api.modrinth.com/").map_err(|error| {
            RemoteContentError::Configuration {
                message: error.to_string(),
            }
        })?;
        Ok(Self::new(base_url))
    }
}

/// Small Modrinth v2 metadata client.
///
/// It is metadata-only: file downloads are handled by `rmcl-net` in the
/// application layer so SHA-1 verification, cancellation and progress remain
/// shared with game downloads.
#[derive(Clone)]
pub struct ModrinthClient {
    client: Client,
    base_url: Url,
}

impl ModrinthClient {
    pub fn new(config: ModrinthConfig) -> Result<Self, RemoteContentError> {
        let client = Client::builder()
            .user_agent(config.user_agent)
            .timeout(config.timeout)
            .build()
            .map_err(|source| RemoteContentError::Http {
                provider: MODRINTH,
                source: Box::new(source),
            })?;
        Ok(Self {
            client,
            base_url: config.base_url,
        })
    }

    pub fn official() -> Result<Self, RemoteContentError> {
        Self::new(ModrinthConfig::official()?)
    }

    pub async fn search(
        &self,
        query: &RemoteSearchQuery,
        cancellation: &CancellationToken,
    ) -> Result<RemoteSearchPage, RemoteContentError> {
        let mut url = self.endpoint("v2/search")?;
        let facets = search_facets(query);
        let facets =
            serde_json::to_string(&facets).map_err(|error| RemoteContentError::Configuration {
                message: error.to_string(),
            })?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("query", &query.query);
            pairs.append_pair("index", query.sort.modrinth_index());
            pairs.append_pair("offset", &query.offset.to_string());
            pairs.append_pair("limit", &query.limit.clamp(1, 100).to_string());
            pairs.append_pair("facets", &facets);
        }
        let response: SearchResponse = self.get_json(url, cancellation).await?;
        Ok(RemoteSearchPage {
            projects: response.hits.into_iter().map(RemoteProject::from).collect(),
            total_hits: response.total_hits,
            offset: response.offset,
            limit: response.limit,
        })
    }

    /// Fetches one version by its Modrinth version ID.
    pub async fn version(
        &self,
        version_id: &str,
        cancellation: &CancellationToken,
    ) -> Result<RemoteVersion, RemoteContentError> {
        let mut url = self.endpoint("v2/version/")?;
        {
            let mut segments =
                url.path_segments_mut()
                    .map_err(|_| RemoteContentError::InvalidResponse {
                        provider: MODRINTH,
                        message: "api base URL cannot contain version path segments".to_owned(),
                    })?;
            segments.pop_if_empty().push(version_id);
        }
        let response: VersionResponse = self.get_json(url, cancellation).await?;
        RemoteVersion::try_from(response)
    }

    /// Fetches one project by ID. Used to map dependency project IDs to slugs
    /// for local mod-ID conflict checks.
    pub async fn project(
        &self,
        project_id: &str,
        cancellation: &CancellationToken,
    ) -> Result<RemoteProject, RemoteContentError> {
        let mut url = self.endpoint("v2/project/")?;
        {
            let mut segments =
                url.path_segments_mut()
                    .map_err(|_| RemoteContentError::InvalidResponse {
                        provider: MODRINTH,
                        message: "api base URL cannot contain project path segments".to_owned(),
                    })?;
            segments.pop_if_empty().push(project_id);
        }
        let response: ProjectResponse = self.get_json(url, cancellation).await?;
        Ok(response.into_project())
    }

    pub async fn project_versions(
        &self,
        project_id: &str,
        game_version: Option<&str>,
        loader: Option<&str>,
        cancellation: &CancellationToken,
    ) -> Result<Vec<RemoteVersion>, RemoteContentError> {
        let mut url = self.endpoint("v2/project/")?;
        {
            let mut segments =
                url.path_segments_mut()
                    .map_err(|_| RemoteContentError::InvalidResponse {
                        provider: MODRINTH,
                        message: "api base URL cannot contain project path segments".to_owned(),
                    })?;
            segments.pop_if_empty().push(project_id).push("version");
        }
        {
            let mut pairs = url.query_pairs_mut();
            if let Some(game_version) = game_version {
                let value = serde_json::to_string(&[game_version]).map_err(|error| {
                    RemoteContentError::Configuration {
                        message: error.to_string(),
                    }
                })?;
                pairs.append_pair("game_versions", &value);
            }
            if let Some(loader) = loader {
                let value = serde_json::to_string(&[loader]).map_err(|error| {
                    RemoteContentError::Configuration {
                        message: error.to_string(),
                    }
                })?;
                pairs.append_pair("loaders", &value);
            }
            pairs.append_pair("include_changelog", "false");
        }
        let response: Vec<VersionResponse> = self.get_json(url, cancellation).await?;
        let mut versions = response
            .into_iter()
            .map(RemoteVersion::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        versions.sort_by(|left, right| right.published.cmp(&left.published));
        Ok(versions)
    }

    fn endpoint(&self, path: &str) -> Result<Url, RemoteContentError> {
        self.base_url
            .join(path)
            .map_err(|error| RemoteContentError::Configuration {
                message: format!("invalid API base URL: {error}"),
            })
    }

    async fn get_json<T: DeserializeOwned>(
        &self,
        url: Url,
        cancellation: &CancellationToken,
    ) -> Result<T, RemoteContentError> {
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(RemoteContentError::Cancelled),
            response = self.client.get(url).send() => response.map_err(|source| {
                RemoteContentError::Http {
                    provider: MODRINTH,
                    source: Box::new(source),
                }
            })?,
        };
        let status = response.status();
        if !status.is_success() {
            let body = tokio::select! {
                _ = cancellation.cancelled() => return Err(RemoteContentError::Cancelled),
                body = response.text() => body.unwrap_or_default(),
            };
            return Err(RemoteContentError::Api {
                provider: MODRINTH,
                status: status.as_u16(),
                message: truncate(&body, 500),
            });
        }
        let body = tokio::select! {
            _ = cancellation.cancelled() => return Err(RemoteContentError::Cancelled),
            body = response.bytes() => body.map_err(|source| RemoteContentError::Http {
                provider: MODRINTH,
                source: Box::new(source),
            })?,
        };
        serde_json::from_slice(&body).map_err(|source| RemoteContentError::Json {
            provider: MODRINTH,
            source: Box::new(source),
        })
    }
}

fn search_facets(query: &RemoteSearchQuery) -> Vec<Vec<String>> {
    let mut facets = vec![vec![format!(
        "project_type:{}",
        query.kind.modrinth_project_type()
    )]];
    if let Some(game_version) = &query.game_version {
        facets.push(vec![format!("versions:{game_version}")]);
    }
    if query.kind == ContentKind::Mod
        && let Some(loader) = &query.loader
    {
        facets.push(vec![format!("categories:{loader}")]);
    }
    facets
}

fn truncate(value: &str, limit: usize) -> String {
    let mut truncated = value.chars().take(limit).collect::<String>();
    if value.chars().count() > limit {
        truncated.push('…');
    }
    truncated
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    hits: Vec<SearchHit>,
    #[serde(default)]
    offset: usize,
    #[serde(default)]
    limit: usize,
    #[serde(default)]
    total_hits: u64,
}

#[derive(Debug, Deserialize)]
struct SearchHit {
    project_id: String,
    project_type: String,
    slug: String,
    #[serde(default)]
    author: String,
    title: String,
    description: String,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    versions: Vec<String>,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    follows: u64,
    #[serde(default)]
    icon_url: Option<String>,
    #[serde(default)]
    date_modified: Option<String>,
}

impl From<SearchHit> for RemoteProject {
    fn from(hit: SearchHit) -> Self {
        let loaders = hit
            .categories
            .iter()
            .filter(|category| {
                matches!(category.as_str(), "fabric" | "quilt" | "forge" | "neoforge")
            })
            .cloned()
            .collect();
        Self {
            source: MODRINTH,
            id: hit.project_id,
            slug: hit.slug,
            title: hit.title,
            description: hit.description,
            author: hit.author,
            icon_url: hit.icon_url,
            downloads: hit.downloads,
            follows: hit.follows,
            categories: hit.categories,
            game_versions: hit.versions,
            loaders,
            project_type: hit.project_type,
            updated: hit.date_modified,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ProjectResponse {
    id: String,
    slug: String,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    categories: Vec<String>,
    project_type: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    follows: u64,
    #[serde(default)]
    icon_url: Option<String>,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    loaders: Vec<String>,
    #[serde(default)]
    updated: Option<String>,
}

impl ProjectResponse {
    fn into_project(self) -> RemoteProject {
        RemoteProject {
            source: MODRINTH,
            id: self.id,
            slug: self.slug,
            title: self.title,
            description: self.description,
            author: String::new(),
            icon_url: self.icon_url,
            downloads: self.downloads,
            follows: self.follows,
            categories: self.categories,
            game_versions: self.game_versions,
            loaders: self.loaders,
            project_type: self.project_type,
            updated: self.updated,
        }
    }
}

#[derive(Debug, Deserialize)]
struct VersionResponse {
    id: String,
    project_id: String,
    name: String,
    version_number: String,
    #[serde(default)]
    changelog: Option<String>,
    #[serde(default)]
    date_published: Option<String>,
    #[serde(default)]
    downloads: u64,
    version_type: String,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    loaders: Vec<String>,
    #[serde(default)]
    files: Vec<VersionFile>,
    #[serde(default)]
    dependencies: Vec<VersionDependency>,
}

#[derive(Debug, Deserialize)]
struct VersionFile {
    #[serde(default)]
    hashes: HashMap<String, String>,
    url: String,
    filename: String,
    #[serde(default)]
    primary: bool,
    #[serde(default)]
    size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct VersionDependency {
    #[serde(default)]
    version_id: Option<String>,
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    dependency_type: String,
}

impl TryFrom<VersionResponse> for RemoteVersion {
    type Error = RemoteContentError;

    fn try_from(version: VersionResponse) -> Result<Self, Self::Error> {
        let files = version
            .files
            .into_iter()
            .map(RemoteFile::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            source: MODRINTH,
            id: version.id,
            project_id: version.project_id,
            name: version.name,
            version_number: version.version_number,
            changelog: version.changelog,
            published: version.date_published,
            version_type: version.version_type,
            downloads: version.downloads,
            game_versions: version.game_versions,
            loaders: version.loaders,
            files,
            dependencies: version
                .dependencies
                .into_iter()
                .map(RemoteDependency::from)
                .collect(),
        })
    }
}

impl TryFrom<VersionFile> for RemoteFile {
    type Error = RemoteContentError;

    fn try_from(file: VersionFile) -> Result<Self, Self::Error> {
        let url = Url::parse(&file.url).map_err(|error| RemoteContentError::InvalidResponse {
            provider: MODRINTH,
            message: format!("invalid file URL for {}: {error}", file.filename),
        })?;
        Ok(Self {
            filename: file.filename,
            url,
            size: file.size,
            sha1: file.hashes.get("sha1").and_then(|hash| parse_sha1(hash)),
            primary: file.primary,
        })
    }
}

impl From<VersionDependency> for RemoteDependency {
    fn from(dependency: VersionDependency) -> Self {
        Self {
            version_id: dependency.version_id,
            project_id: dependency.project_id,
            file_name: dependency.file_name,
            kind: RemoteDependencyKind::from_api(&dependency.dependency_type),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc, time::Duration};

    use axum::{Json, Router, extract::Query, http::StatusCode, routing::get};
    use serde_json::json;
    use tokio::sync::Mutex;

    use super::*;
    use crate::{ContentKind, RemoteSort};

    async fn server(router: Router) -> (Url, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (base, handle)
    }

    fn client(base: Url) -> ModrinthClient {
        ModrinthClient::new(ModrinthConfig {
            base_url: base,
            user_agent: "RMCL-test".to_owned(),
            timeout: Duration::from_secs(5),
        })
        .unwrap()
    }

    #[tokio::test]
    async fn search_builds_facets_and_parses_projects() {
        let captured = Arc::new(Mutex::new(Vec::<HashMap<String, String>>::new()));
        let handler_captured = captured.clone();
        let router = Router::new().route(
            "/v2/search",
            get(move |Query(query): Query<HashMap<String, String>>| {
                let captured = handler_captured.clone();
                async move {
                    captured.lock().await.push(query);
                    Json(json!({
                        "hits": [{
                            "project_id": "sodium",
                            "project_type": "mod",
                            "slug": "sodium",
                            "author": "jellysquid3",
                            "title": "Sodium",
                            "description": "Modern rendering engine",
                            "categories": ["fabric", "optimization"],
                            "versions": ["1.21.1"],
                            "downloads": 1000,
                            "follows": 42,
                            "icon_url": "https://cdn.modrinth.com/sodium.png",
                            "date_modified": "2026-01-01T00:00:00Z"
                        }],
                        "offset": 0,
                        "limit": 20,
                        "total_hits": 1
                    }))
                }
            }),
        );
        let (base, handle) = server(router).await;
        let client = client(base);
        let mut query = RemoteSearchQuery::new(ContentKind::Mod);
        query.query = "sodium".to_owned();
        query.game_version = Some("1.21.1".to_owned());
        query.loader = Some("fabric".to_owned());
        query.sort = RemoteSort::Downloads;

        let page = client
            .search(&query, &CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(page.total_hits, 1);
        assert_eq!(page.projects.len(), 1);
        assert_eq!(page.projects[0].title, "Sodium");
        assert_eq!(page.projects[0].author, "jellysquid3");
        assert_eq!(page.projects[0].loaders, ["fabric"]);
        let captured = captured.lock().await;
        let params = &captured[0];
        assert_eq!(params.get("query").map(String::as_str), Some("sodium"));
        assert_eq!(params.get("index").map(String::as_str), Some("downloads"));
        let facets = params.get("facets").unwrap();
        assert!(facets.contains("project_type:mod"));
        assert!(facets.contains("versions:1.21.1"));
        assert!(facets.contains("categories:fabric"));
        handle.abort();
    }

    #[tokio::test]
    async fn versions_parse_files_dependencies_and_sort_newest_first() {
        let router = Router::new().route(
            "/v2/project/sodium/version",
            get(|| async {
                Json(json!([
                    {
                        "id": "v-old",
                        "project_id": "sodium",
                        "name": "Sodium 0.5",
                        "version_number": "0.5.0",
                        "date_published": "2025-01-01T00:00:00Z",
                        "version_type": "release",
                        "game_versions": ["1.21.1"],
                        "loaders": ["fabric"],
                        "files": [{
                            "hashes": {"sha1": "0123456789abcdef0123456789abcdef01234567", "sha512": "ignored"},
                            "url": "https://cdn.modrinth.com/old.jar",
                            "filename": "sodium-old.jar",
                            "primary": true,
                            "size": 10
                        }],
                        "dependencies": []
                    },
                    {
                        "id": "v-new",
                        "project_id": "sodium",
                        "name": "Sodium 0.6",
                        "version_number": "0.6.0",
                        "date_published": "2026-01-01T00:00:00Z",
                        "version_type": "beta",
                        "game_versions": ["1.21.1"],
                        "loaders": ["fabric"],
                        "files": [
                            {
                                "hashes": {"sha1": "89abcdef0123456789abcdef0123456789abcdef"},
                                "url": "https://cdn.modrinth.com/sources.jar",
                                "filename": "sodium-sources.jar",
                                "primary": false,
                                "size": 20
                            },
                            {
                                "hashes": {"sha1": "fedcba9876543210fedcba9876543210fedcba98"},
                                "url": "https://cdn.modrinth.com/new.jar",
                                "filename": "sodium-new.jar",
                                "primary": true,
                                "size": 30
                            }
                        ],
                        "dependencies": [{
                            "project_id": "fabric-api",
                            "version_id": null,
                            "file_name": null,
                            "dependency_type": "required"
                        }]
                    }
                ]))
            }),
        );
        let (base, handle) = server(router).await;
        let client = client(base);

        let versions = client
            .project_versions(
                "sodium",
                Some("1.21.1"),
                Some("fabric"),
                &CancellationToken::new(),
            )
            .await
            .unwrap();

        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].id, "v-new");
        assert_eq!(versions[0].version_type, "beta");
        assert_eq!(versions[0].required_dependencies().count(), 1);
        let file = versions[0].installable_file(ContentKind::Mod).unwrap();
        assert_eq!(file.filename, "sodium-new.jar");
        assert_eq!(file.size, Some(30));
        assert!(file.sha1.is_some());
        handle.abort();
    }

    #[tokio::test]
    async fn fetches_one_version_by_id() {
        let router = Router::new().route(
            "/v2/version/dep-version",
            get(|| async {
                Json(json!({
                    "id": "dep-version",
                    "project_id": "fabric-api",
                    "name": "Fabric API 0.100.0",
                    "version_number": "0.100.0",
                    "version_type": "release",
                    "game_versions": ["1.21.1"],
                    "loaders": ["fabric"],
                    "files": [{
                        "hashes": {"sha1": "0123456789abcdef0123456789abcdef01234567"},
                        "url": "https://cdn.modrinth.com/fabric-api.jar",
                        "filename": "fabric-api.jar",
                        "primary": true,
                        "size": 42
                    }],
                    "dependencies": []
                }))
            }),
        );
        let (base, handle) = server(router).await;
        let client = client(base);

        let version = client
            .version("dep-version", &CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(version.id, "dep-version");
        assert_eq!(version.name, "Fabric API 0.100.0");
        assert_eq!(
            version
                .installable_file(ContentKind::Mod)
                .map(|file| file.filename.as_str()),
            Some("fabric-api.jar")
        );
        handle.abort();
    }

    #[tokio::test]
    async fn fetches_one_project_by_id_for_dependency_preflight() {
        let router = Router::new().route(
            "/v2/project/P7dR8mSH",
            get(|| async {
                Json(json!({
                    "id": "P7dR8mSH",
                    "slug": "fabric-api",
                    "title": "Fabric API",
                    "description": "Core API",
                    "categories": ["fabric", "library"],
                    "project_type": "mod",
                    "downloads": 1000,
                    "follows": 42,
                    "icon_url": "https://cdn.modrinth.com/fabric-api.png",
                    "game_versions": ["1.21.1"],
                    "loaders": ["fabric"],
                    "updated": "2026-01-01T00:00:00Z"
                }))
            }),
        );
        let (base, handle) = server(router).await;
        let client = client(base);

        let project = client
            .project("P7dR8mSH", &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(project.slug, "fabric-api");
        assert_eq!(project.title, "Fabric API");
        handle.abort();
    }

    #[tokio::test]
    async fn api_errors_are_structured() {
        let router = Router::new().route(
            "/v2/search",
            get(|| async { (StatusCode::NOT_FOUND, "missing project") }),
        );
        let (base, handle) = server(router).await;
        let client = client(base);

        let error = client
            .search(
                &RemoteSearchQuery::new(ContentKind::Mod),
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, RemoteContentError::Api { status: 404, .. }));
        handle.abort();
    }

    #[tokio::test]
    async fn cancellation_stops_an_in_flight_request() {
        let router = Router::new().route(
            "/v2/search",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Json(json!({"hits": [], "offset": 0, "limit": 20, "total_hits": 0}))
            }),
        );
        let (base, handle) = server(router).await;
        let client = client(base);
        let cancellation = CancellationToken::new();
        let cancel = cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            cancel.cancel();
        });

        let error = client
            .search(&RemoteSearchQuery::new(ContentKind::Mod), &cancellation)
            .await
            .unwrap_err();

        assert!(matches!(error, RemoteContentError::Cancelled));
        handle.abort();
    }
}
