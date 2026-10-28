use std::{collections::BTreeSet, sync::Arc};

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use rmcl_app::AppContext;
use rmcp::{
    ServiceExt,
    transport::{
        StreamableHttpServerConfig, StreamableHttpService, stdio,
        streamable_http_server::session::local::LocalSessionManager,
    },
};
use subtle::ConstantTimeEq;
use tokio_util::sync::CancellationToken;
use tower::ServiceBuilder;

use crate::{AccessLevel, error::McpServerError, server::McpServer};

#[derive(Debug, Clone)]
pub struct HttpServerConfig {
    pub port: u16,
    pub access: AccessLevel,
    pub bearer_token: Option<String>,
    pub allowed_origins: BTreeSet<String>,
}

impl HttpServerConfig {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            access: AccessLevel::ReadOnly,
            bearer_token: None,
            allowed_origins: BTreeSet::new(),
        }
    }
}

#[derive(Clone)]
struct HttpSecurity {
    bearer_token: Option<Arc<str>>,
    allowed_origins: Arc<BTreeSet<String>>,
    port: u16,
}

impl From<HttpServerConfig> for HttpSecurity {
    fn from(config: HttpServerConfig) -> Self {
        Self {
            bearer_token: config.bearer_token.map(Arc::from),
            allowed_origins: Arc::new(config.allowed_origins),
            port: config.port,
        }
    }
}

pub async fn serve_stdio(context: Arc<AppContext>) -> Result<(), McpServerError> {
    let running = McpServer::new(context)
        .serve(stdio())
        .await
        .map_err(|error| McpServerError::Serve(error.to_string()))?;
    let _ = running
        .waiting()
        .await
        .map_err(|error| McpServerError::Serve(error.to_string()))?;
    Ok(())
}

/// A running loopback HTTP server whose shutdown can be requested and awaited.
pub struct HttpServerHandle {
    cancellation: CancellationToken,
    worker: Option<tokio::task::JoinHandle<Result<(), McpServerError>>>,
}

impl HttpServerHandle {
    pub async fn start(
        context: Arc<AppContext>,
        config: HttpServerConfig,
        cancellation: CancellationToken,
    ) -> Result<Self, McpServerError> {
        if config.port == 0 {
            return Err(McpServerError::Serve(
                "MCP port must be between 1 and 65535".to_owned(),
            ));
        }
        let access = config.access;
        let port = config.port;
        let security = HttpSecurity::from(config);
        let service_context = context.clone();
        let config = StreamableHttpServerConfig::default()
            .with_cancellation_token(cancellation.clone())
            .with_json_response(true);
        let service = StreamableHttpService::new(
            move || Ok(McpServer::with_access(service_context.clone(), access)),
            Arc::new(LocalSessionManager::default()),
            config,
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
        let service = <StreamableHttpService<_, _> as tower::ServiceExt<
            axum::http::Request<axum::body::Body>,
        >>::map_response(service, |response: axum::http::Response<_>| {
            response.map(axum::body::Body::new)
        });
        let service = ServiceBuilder::new()
            .layer(axum::middleware::from_fn_with_state(
                security,
                authorize_http,
            ))
            .service(service);
        let shutdown = cancellation.clone().cancelled_owned();
        let worker = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::ServiceExt::<axum::http::Request<axum::body::Body>>::into_make_service(
                    service,
                ),
            )
            .with_graceful_shutdown(shutdown)
            .await
            .map_err(|error| McpServerError::Serve(error.to_string()))
        });
        Ok(Self {
            cancellation,
            worker: Some(worker),
        })
    }

    pub async fn stop(mut self) -> Result<(), McpServerError> {
        self.cancellation.cancel();
        self.wait_worker().await
    }

    pub async fn wait(mut self) -> Result<(), McpServerError> {
        self.wait_worker().await
    }

    async fn wait_worker(&mut self) -> Result<(), McpServerError> {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        worker
            .await
            .map_err(|error| McpServerError::Serve(error.to_string()))?
    }
}

impl Drop for HttpServerHandle {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

pub async fn serve_http(
    context: Arc<AppContext>,
    config: HttpServerConfig,
    cancellation: CancellationToken,
) -> Result<(), McpServerError> {
    HttpServerHandle::start(context, config, cancellation)
        .await?
        .wait()
        .await
}

async fn authorize_http(
    axum::extract::State(security): axum::extract::State<HttpSecurity>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, HttpRejection> {
    validate_http_headers(&security, request.headers())?;
    Ok(next.run(request).await)
}

fn validate_http_headers(
    security: &HttpSecurity,
    headers: &HeaderMap,
) -> Result<(), HttpRejection> {
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_ascii_lowercase);
    let valid_host = host.is_some_and(|host| {
        host == format!("127.0.0.1:{}", security.port)
            || host == format!("localhost:{}", security.port)
            || host == format!("[::1]:{}", security.port)
    });
    if !valid_host {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "loopback Host header required",
        ));
    }

    let mut origins = headers.get_all(header::ORIGIN).iter();
    if let Some(origin) = origins.next()
        && (origins.next().is_some()
            || !origin
                .to_str()
                .ok()
                .is_some_and(|origin| security.allowed_origins.contains(origin)))
    {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "Origin is not allowed",
        ));
    }

    if let Some(expected) = &security.bearer_token {
        let expected = format!("Bearer {expected}");
        let authorized = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|provided| bool::from(provided.as_bytes().ct_eq(expected.as_bytes())));
        if !authorized {
            let mut rejection =
                error_response(StatusCode::UNAUTHORIZED, "valid Bearer token required");
            rejection.bearer_challenge = true;
            return Err(rejection);
        }
    }
    Ok(())
}

#[derive(Debug)]
struct HttpRejection {
    status: StatusCode,
    message: &'static str,
    bearer_challenge: bool,
}

impl IntoResponse for HttpRejection {
    fn into_response(self) -> Response {
        let mut response = (self.status, self.message).into_response();
        if self.bearer_challenge {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Bearer realm=\"rmcl-mcp\""),
            );
        }
        response
    }
}

fn error_response(status: StatusCode, message: &'static str) -> HttpRejection {
    HttpRejection {
        status,
        message,
        bearer_challenge: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn security(token: Option<&str>) -> HttpSecurity {
        HttpSecurity {
            bearer_token: token.map(Arc::from),
            allowed_origins: Arc::new(BTreeSet::from(["https://agent.example".to_owned()])),
            port: 39271,
        }
    }

    fn headers(host: &str, origin: Option<&str>, authorization: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_str(host).unwrap());
        if let Some(origin) = origin {
            headers.insert(header::ORIGIN, HeaderValue::from_str(origin).unwrap());
        }
        if let Some(authorization) = authorization {
            headers.insert(
                header::AUTHORIZATION,
                HeaderValue::from_str(authorization).unwrap(),
            );
        }
        headers
    }

    #[test]
    fn loopback_hosts_and_non_browser_requests_are_accepted() {
        for host in ["127.0.0.1:39271", "localhost:39271", "[::1]:39271"] {
            assert!(validate_http_headers(&security(None), &headers(host, None, None)).is_ok());
        }
        assert!(
            validate_http_headers(&security(None), &headers("example.com", None, None)).is_err()
        );
        assert!(
            validate_http_headers(
                &security(None),
                &headers("127.0.0.1:39271", Some("https://evil.example"), None)
            )
            .is_err()
        );
    }

    #[test]
    fn origin_allowlist_is_explicit() {
        let security = security(None);
        assert!(
            validate_http_headers(
                &security,
                &headers("localhost:39271", Some("https://agent.example"), None)
            )
            .is_ok()
        );
        assert!(
            validate_http_headers(
                &security,
                &headers("localhost:39271", Some("https://evil.example"), None)
            )
            .is_err()
        );
    }

    #[test]
    fn bearer_token_is_required_and_compared_exactly() {
        let security = security(Some("secret"));
        assert!(
            validate_http_headers(
                &security,
                &headers("localhost:39271", None, Some("Bearer secret"))
            )
            .is_ok()
        );
        assert!(
            validate_http_headers(
                &security,
                &headers("localhost:39271", None, Some("Bearer wrong"))
            )
            .is_err()
        );
        assert!(validate_http_headers(&security, &headers("localhost:39271", None, None)).is_err());
    }

    #[test]
    fn http_defaults_are_read_only_and_have_no_origins() {
        let config = HttpServerConfig::new(39271);
        assert_eq!(config.access, AccessLevel::ReadOnly);
        assert!(config.allowed_origins.is_empty());
        assert!(config.bearer_token.is_none());
    }
}
