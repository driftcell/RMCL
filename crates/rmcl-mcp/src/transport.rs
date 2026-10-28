use std::sync::Arc;

use rmcl_app::AppContext;
use rmcp::{
    ServiceExt,
    transport::{
        StreamableHttpServerConfig, StreamableHttpService, stdio,
        streamable_http_server::session::local::LocalSessionManager,
    },
};
use tokio_util::sync::CancellationToken;

use crate::{error::McpServerError, server::McpServer};

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

pub async fn serve_http(
    context: Arc<AppContext>,
    port: u16,
    cancellation: CancellationToken,
) -> Result<(), McpServerError> {
    let service_context = context.clone();
    let config = StreamableHttpServerConfig::default()
        .with_cancellation_token(cancellation.clone())
        .with_json_response(true);
    let service = StreamableHttpService::new(
        move || Ok(McpServer::new(service_context.clone())),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let service = <StreamableHttpService<_, _> as tower::ServiceExt<
        axum::http::Request<axum::body::Body>,
    >>::map_response(service, |response: axum::http::Response<_>| {
        response.map(axum::body::Body::new)
    });
    axum::serve(
        listener,
        axum::ServiceExt::<axum::http::Request<axum::body::Body>>::into_make_service(service),
    )
    .with_graceful_shutdown(cancellation.cancelled_owned())
    .await
    .map_err(|error| McpServerError::Serve(error.to_string()))
}
