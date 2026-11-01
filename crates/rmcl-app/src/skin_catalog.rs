//! Online skin catalog backed by a Blessing Skin compatible site.
//!
//! The first provider is LittleSkin. It exposes the same JSON endpoints used by
//! its own skin library page; this is a site adapter, not a Mojang API.

use std::time::Duration;

use reqwest::{StatusCode, header::ACCEPT};
use rmcl_auth::SkinVariant;
use serde::Deserialize;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::skin_source::{PreparedSkin, SkinSourceError, SkinUrlClient};

/// The site used by the built-in catalog provider.
pub const LITTLESKIN_BASE_URL: &str = "https://littleskin.cn";

const MAX_CATALOG_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_SKIN_INFO_BYTES: usize = 64 * 1024;

/// The order used when listing catalog skins.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SkinCatalogSort {
    /// Most liked first.
    #[default]
    Popular,
    /// Newest upload first.
    Newest,
}

impl SkinCatalogSort {
    pub const ALL: [Self; 2] = [Self::Popular, Self::Newest];

    pub fn label(self) -> &'static str {
        match self {
            Self::Popular => "热门",
            Self::Newest => "最新",
        }
    }

    fn query_value(self) -> &'static str {
        match self {
            Self::Popular => "likes",
            Self::Newest => "time",
        }
    }
}

/// One skin in the online catalog.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinCatalogItem {
    pub id: String,
    pub tid: u64,
    pub name: String,
    pub author: Option<String>,
    pub variant: SkinVariant,
    pub likes: u64,
    pub site_url: String,
}

/// A page of catalog results.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct SkinCatalogPage {
    pub items: Vec<SkinCatalogItem>,
    pub page: u32,
    pub has_more: bool,
}

/// A catalog skin whose PNG has been downloaded and validated for preview.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct CatalogSkin {
    pub item: SkinCatalogItem,
    pub skin: PreparedSkin,
}

impl CatalogSkin {
    pub fn new(item: SkinCatalogItem, skin: PreparedSkin) -> Self {
        Self { item, skin }
    }
}

impl SkinCatalogItem {
    pub fn new(
        id: impl Into<String>,
        tid: u64,
        name: impl Into<String>,
        author: Option<String>,
        variant: SkinVariant,
        likes: u64,
        site_url: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            tid,
            name: name.into(),
            author,
            variant,
            likes,
            site_url: site_url.into(),
        }
    }
}

/// Fetches LittleSkin's public skin library.
#[derive(Clone)]
pub struct SkinCatalogClient {
    base: Url,
    client: reqwest::Client,
}

impl SkinCatalogClient {
    pub fn new() -> Result<Self, SkinCatalogError> {
        let base = Url::parse(LITTLESKIN_BASE_URL)?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(3))
            .timeout(Duration::from_secs(20))
            .user_agent(concat!(
                "RMCL/",
                env!("CARGO_PKG_VERSION"),
                " (skin catalog)"
            ))
            .build()
            .map_err(SkinCatalogError::Http)?;
        Ok(Self { base, client })
    }

    /// Lists skins, optionally filtered by a name keyword.
    pub async fn list(
        &self,
        query: &str,
        sort: SkinCatalogSort,
        page: u32,
        cancellation: &CancellationToken,
    ) -> Result<SkinCatalogPage, SkinCatalogError> {
        let mut url = self.base.join("skinlib/list")?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("filter", "skin");
            pairs.append_pair("sort", sort.query_value());
            pairs.append_pair("page", &page.max(1).to_string());
            let query = query.trim();
            if !query.is_empty() {
                pairs.append_pair("keyword", query);
            }
        }

        let request = self.client.get(url).header(ACCEPT, "application/json");
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(SkinCatalogError::Cancelled),
            response = request.send() => response.map_err(SkinCatalogError::Http)?,
        };
        let status = response.status();
        if !status.is_success() {
            return Err(SkinCatalogError::HttpStatus { status });
        }
        let bytes = read_limited(response, MAX_CATALOG_RESPONSE_BYTES, cancellation).await?;
        let response: ListResponse = serde_json::from_slice(&bytes)?;
        let has_more = response.next_page_url.is_some();
        let items = response
            .data
            .into_iter()
            .filter(|item| item.public && !item.hd)
            .filter_map(|item| item.into_catalog_item(&self.base).transpose())
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SkinCatalogPage {
            items,
            page: response.current_page.max(1),
            has_more,
        })
    }

    /// Resolves the item's texture and prepares it for preview and upload.
    ///
    /// Catalog listings only expose a numeric texture id; the hash and texture
    /// URL are fetched from the item detail endpoint.
    pub async fn prepare(
        &self,
        item: &SkinCatalogItem,
        urls: &SkinUrlClient,
        cancellation: &CancellationToken,
    ) -> Result<PreparedSkin, SkinCatalogError> {
        let url = self.base.join(&format!("skinlib/info/{}", item.tid))?;
        let request = self.client.get(url).header(ACCEPT, "application/json");
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(SkinCatalogError::Cancelled),
            response = request.send() => response.map_err(SkinCatalogError::Http)?,
        };
        let status = response.status();
        if !status.is_success() {
            return Err(SkinCatalogError::HttpStatus { status });
        }
        let bytes = read_limited(response, MAX_SKIN_INFO_BYTES, cancellation).await?;
        let info: SkinInfo = serde_json::from_slice(&bytes)?;
        let hash = validate_texture_hash(&info.hash)?;
        let texture_url = self.base.join(&format!("textures/{hash}"))?;
        let skin = urls.fetch(texture_url.as_str(), cancellation).await?;
        Ok(skin
            .with_file_name(format!("{}.png", item.name))
            .with_source_url(Some(item.site_url.clone())))
    }

    #[cfg(test)]
    fn for_test(base: Url) -> Result<Self, SkinCatalogError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(SkinCatalogError::Http)?;
        Ok(Self { base, client })
    }
}

#[derive(Debug, Deserialize)]
struct ListResponse {
    #[serde(default = "default_page")]
    current_page: u32,
    #[serde(default)]
    data: Vec<ListItem>,
    #[serde(default)]
    next_page_url: Option<String>,
}

const fn default_page() -> u32 {
    1
}

#[derive(Debug, Deserialize)]
struct ListItem {
    #[serde(default)]
    tid: u64,
    #[serde(default)]
    name: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    public: bool,
    #[serde(default)]
    hd: bool,
    #[serde(default)]
    likes: u64,
    #[serde(default)]
    nickname: String,
}

impl ListItem {
    fn into_catalog_item(self, base: &Url) -> Result<Option<SkinCatalogItem>, SkinCatalogError> {
        let Some(variant) = variant_from_kind(&self.kind) else {
            return Ok(None);
        };
        if self.tid == 0 {
            return Ok(None);
        }
        let name = if self.name.trim().is_empty() {
            "未命名皮肤".to_owned()
        } else {
            self.name.trim().to_owned()
        };
        let site_url = base.join(&format!("skinlib/show/{}", self.tid))?;
        Ok(Some(SkinCatalogItem {
            id: format!("littleskin:{}", self.tid),
            tid: self.tid,
            name,
            author: (!self.nickname.trim().is_empty()).then(|| self.nickname.trim().to_owned()),
            variant,
            likes: self.likes,
            site_url: site_url.to_string(),
        }))
    }
}

#[derive(Debug, Deserialize)]
struct SkinInfo {
    #[serde(default)]
    hash: String,
}

fn variant_from_kind(kind: &str) -> Option<SkinVariant> {
    match kind {
        "steve" => Some(SkinVariant::Classic),
        "alex" => Some(SkinVariant::Slim),
        _ => None,
    }
}

fn validate_texture_hash(hash: &str) -> Result<&str, SkinCatalogError> {
    if hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(hash)
    } else {
        Err(SkinCatalogError::InvalidTextureHash)
    }
}

async fn read_limited(
    mut response: reqwest::Response,
    limit: usize,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, SkinCatalogError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(SkinCatalogError::ResponseTooLarge { limit });
    }
    let mut bytes = Vec::new();
    loop {
        let chunk = tokio::select! {
            _ = cancellation.cancelled() => return Err(SkinCatalogError::Cancelled),
            chunk = response.chunk() => chunk.map_err(SkinCatalogError::Http)?,
        };
        let Some(chunk) = chunk else {
            break;
        };
        if bytes.len() + chunk.len() > limit {
            return Err(SkinCatalogError::ResponseTooLarge { limit });
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[derive(Debug, Error)]
pub enum SkinCatalogError {
    #[error("invalid skin catalog URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("skin catalog request failed: {0}")]
    Http(#[source] reqwest::Error),
    #[error("skin catalog returned HTTP {status}")]
    HttpStatus { status: StatusCode },
    #[error("skin catalog response exceeds the {limit} byte limit")]
    ResponseTooLarge { limit: usize },
    #[error("skin catalog returned invalid JSON: {0}")]
    InvalidResponse(#[from] serde_json::Error),
    #[error("skin catalog returned an invalid texture hash")]
    InvalidTextureHash,
    #[error(transparent)]
    Source(#[from] SkinSourceError),
    #[error("skin catalog request was cancelled")]
    Cancelled,
    #[error("skin catalog processing task failed: {0}")]
    Blocking(String),
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use axum::{Router, extract::State, response::IntoResponse, routing::get};
    use image::{Rgba, RgbaImage};
    use reqwest::header::CONTENT_TYPE;
    use tokio::net::TcpListener;

    use super::*;

    fn png(color: [u8; 4]) -> Vec<u8> {
        let image = RgbaImage::from_pixel(64, 64, Rgba(color));
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    async fn mock_server(state: Arc<AtomicUsize>) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new()
            .route(
                "/skinlib/list",
                get(|State(requests): State<Arc<AtomicUsize>>| async move {
                    requests.fetch_add(1, Ordering::SeqCst);
                    (
                        [(CONTENT_TYPE, "application/json")],
                        r#"{
                            "current_page": 1,
                            "data": [
                                {"tid": 1, "name": "Steve", "type": "steve", "public": true, "hd": false, "likes": 12, "nickname": "alice"},
                                {"tid": 2, "name": "Alex", "type": "alex", "public": true, "hd": false, "likes": 7, "nickname": "bob"},
                                {"tid": 3, "name": "HD", "type": "steve", "public": true, "hd": true, "likes": 1, "nickname": "carol"},
                                {"tid": 4, "name": "Cape", "type": "cape", "public": true, "hd": false, "likes": 99, "nickname": "dave"}
                            ],
                            "next_page_url": "http://example.invalid/skinlib/list?page=2"
                        }"#,
                    )
                        .into_response()
                }),
            )
            .route(
                "/skinlib/info/1",
                get(|State(requests): State<Arc<AtomicUsize>>| async move {
                    requests.fetch_add(1, Ordering::SeqCst);
                    (
                        [(CONTENT_TYPE, "application/json")],
                        format!(r#"{{"hash":"{}","type":"steve","hd":false}}"#, "a".repeat(64)),
                    )
                        .into_response()
                }),
            )
            .route(
                "/textures/{hash}",
                get(
                    |State(requests): State<Arc<AtomicUsize>>, axum::extract::Path(_): axum::extract::Path<String>| async move {
                        requests.fetch_add(1, Ordering::SeqCst);
                        (
                            [(CONTENT_TYPE, "image/png")],
                            png([10, 20, 30, 255]),
                        )
                            .into_response()
                    },
                ),
            )
            .with_state(state);
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{address}"), server)
    }

    #[derive(Default)]
    struct ListCapture {
        uri: Mutex<Option<String>>,
    }

    async fn capture_list_server(
        capture: Arc<ListCapture>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new()
            .route(
                "/skinlib/list",
                get(
                    |State(capture): State<Arc<ListCapture>>, uri: axum::http::Uri| async move {
                        *capture.uri.lock().unwrap() = Some(uri.to_string());
                        (
                            [(CONTENT_TYPE, "application/json")],
                            r#"{
                                "current_page": 3,
                                "data": [
                                    {"tid": 5, "name": "Public", "type": "steve", "public": true, "hd": false, "likes": 3, "nickname": "alice"},
                                    {"tid": 6, "name": "Private", "type": "steve", "public": false, "hd": false, "likes": 2, "nickname": "bob"},
                                    {"tid": 7, "name": "HD", "type": "steve", "public": true, "hd": true, "likes": 1, "nickname": "carol"}
                                ],
                                "next_page_url": null
                            }"#,
                        )
                            .into_response()
                    },
                ),
            )
            .with_state(capture);
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{address}"), server)
    }

    #[tokio::test]
    async fn lists_and_filters_littleskin_skins() {
        let requests = Arc::new(AtomicUsize::new(0));
        let (base, server) = mock_server(requests.clone()).await;
        let client = SkinCatalogClient::for_test(Url::parse(&base).unwrap()).unwrap();
        let page = client
            .list(
                "steve",
                SkinCatalogSort::Popular,
                1,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].id, "littleskin:1");
        assert_eq!(page.items[0].variant, SkinVariant::Classic);
        assert_eq!(page.items[1].variant, SkinVariant::Slim);
        assert!(page.has_more);
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn sends_search_paging_and_sort_parameters_and_filters_private_entries() {
        let capture = Arc::new(ListCapture::default());
        let (base, server) = capture_list_server(capture.clone()).await;
        let client = SkinCatalogClient::for_test(Url::parse(&base).unwrap()).unwrap();

        let page = client
            .list(
                "foo bar",
                SkinCatalogSort::Newest,
                3,
                &CancellationToken::new(),
            )
            .await
            .unwrap();

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, "littleskin:5");
        assert!(!page.has_more);

        let uri = capture.uri.lock().unwrap().clone().unwrap();
        let url = Url::parse(&format!("{base}{uri}")).unwrap();
        let pairs = url.query_pairs().into_owned().collect::<Vec<_>>();
        assert!(pairs.contains(&("filter".to_owned(), "skin".to_owned())));
        assert!(pairs.contains(&("sort".to_owned(), "time".to_owned())));
        assert!(pairs.contains(&("page".to_owned(), "3".to_owned())));
        assert!(pairs.contains(&("keyword".to_owned(), "foo bar".to_owned())));
        server.abort();
    }

    #[tokio::test]
    async fn prepares_texture_from_detail_hash() {
        let requests = Arc::new(AtomicUsize::new(0));
        let (base, server) = mock_server(requests.clone()).await;
        let client = SkinCatalogClient::for_test(Url::parse(&base).unwrap()).unwrap();
        let item = SkinCatalogItem {
            id: "littleskin:1".to_owned(),
            tid: 1,
            name: "Steve".to_owned(),
            author: Some("alice".to_owned()),
            variant: SkinVariant::Classic,
            likes: 12,
            site_url: format!("{base}/skinlib/show/1"),
        };
        let skin = client
            .prepare(&item, &SkinUrlClient::for_test(), &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(skin.file_name(), "Steve.png");
        assert_eq!(skin.variant_hint(), SkinVariant::Classic);
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        server.abort();
    }

    #[tokio::test]
    async fn rejects_non_hex_texture_hash() {
        let hash = "z".repeat(64);
        assert!(matches!(
            validate_texture_hash(&hash),
            Err(SkinCatalogError::InvalidTextureHash)
        ));
    }

    #[test]
    fn maps_blessing_skin_types() {
        assert_eq!(variant_from_kind("steve"), Some(SkinVariant::Classic));
        assert_eq!(variant_from_kind("alex"), Some(SkinVariant::Slim));
        assert_eq!(variant_from_kind("cape"), None);
    }
}
