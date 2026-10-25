use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use rmcl_domain::{DownloadRequest, Sha1Hash};
use rmcl_net::{
    DownloadConfig, DownloadError, DownloadExecutor, DownloadStatus, ProgressStage,
    progress_channel,
};
use sha1::{Digest, Sha1};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use url::Url;

struct TestServer {
    base: Url,
    handle: tokio::task::JoinHandle<()>,
}

impl TestServer {
    fn url(&self, path: &str) -> Url {
        self.base.join(path).unwrap()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn serve(router: Router) -> TestServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            eprintln!("test server failed: {error}");
        }
    });
    TestServer {
        base: Url::parse(&format!("http://{address}/")).unwrap(),
        handle,
    }
}

fn config() -> DownloadConfig {
    DownloadConfig {
        max_concurrency: 2,
        max_concurrency_per_host: 2,
        max_attempts_per_source: 3,
        connect_timeout: Duration::from_secs(1),
        request_timeout: Duration::from_secs(2),
        initial_retry_delay: Duration::from_millis(1),
        max_retry_delay: Duration::from_millis(5),
    }
}

fn executor() -> DownloadExecutor {
    DownloadExecutor::new(config()).unwrap()
}

fn sha1(data: &[u8]) -> Sha1Hash {
    let digest = Sha1::digest(data);
    let mut bytes = [0_u8; 20];
    bytes.copy_from_slice(&digest);
    Sha1Hash::from_bytes(bytes)
}

fn request(source: Url, destination: PathBuf, expected: &[u8]) -> DownloadRequest {
    DownloadRequest {
        sources: vec![source],
        destination,
        sha1: Some(sha1(expected)),
        size: Some(expected.len() as u64),
    }
}

fn entry_count(directory: &TempDir) -> usize {
    std::fs::read_dir(directory.path()).unwrap().count()
}

#[tokio::test]
async fn downloads_and_verifies_a_file() {
    let server = serve(Router::new().route("/file", get(|| async { "hello" }))).await;
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("nested/file.txt");
    let request = request(server.url("/file"), destination.clone(), b"hello");

    let status = executor()
        .download_file(&request, CancellationToken::new(), None)
        .await
        .unwrap();

    assert_eq!(status, DownloadStatus::Downloaded { bytes: 5 });
    assert_eq!(std::fs::read(destination).unwrap(), b"hello");
}

#[tokio::test]
async fn skips_a_verified_existing_file_without_http() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed = hits.clone();
    let server = serve(Router::new().route(
        "/file",
        get(move || {
            let observed = observed.clone();
            async move {
                observed.fetch_add(1, Ordering::SeqCst);
                "hello"
            }
        }),
    ))
    .await;
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("file.txt");
    std::fs::write(&destination, b"hello").unwrap();
    let request = request(server.url("/file"), destination, b"hello");

    let status = executor()
        .download_file(&request, CancellationToken::new(), None)
        .await
        .unwrap();

    assert_eq!(status, DownloadStatus::Skipped { bytes: 5 });
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn replaces_a_stale_existing_file() {
    let server = serve(Router::new().route("/file", get(|| async { "hello" }))).await;
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("file.txt");
    std::fs::write(&destination, b"old").unwrap();
    let request = request(server.url("/file"), destination.clone(), b"hello");

    executor()
        .download_file(&request, CancellationToken::new(), None)
        .await
        .unwrap();

    assert_eq!(std::fs::read(destination).unwrap(), b"hello");
}

#[tokio::test]
async fn preserves_old_file_when_every_candidate_fails() {
    let server = serve(Router::new().route("/file", get(|| async { "evil" }))).await;
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("file.txt");
    std::fs::write(&destination, b"old").unwrap();
    let request = request(server.url("/file"), destination.clone(), b"good");

    let error = executor()
        .download_file(&request, CancellationToken::new(), None)
        .await
        .unwrap_err();

    assert!(matches!(error, DownloadError::Exhausted { .. }));
    assert_eq!(std::fs::read(destination).unwrap(), b"old");
    assert_eq!(entry_count(&temporary), 1);
}

#[tokio::test]
async fn retries_hash_mismatch_without_publishing() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed = hits.clone();
    let server = serve(Router::new().route(
        "/file",
        get(move || {
            let observed = observed.clone();
            async move {
                observed.fetch_add(1, Ordering::SeqCst);
                "evil"
            }
        }),
    ))
    .await;
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("file.txt");
    let request = request(server.url("/file"), destination.clone(), b"good");

    let error = executor()
        .download_file(&request, CancellationToken::new(), None)
        .await
        .unwrap_err();

    assert!(matches!(
        &error,
        DownloadError::Exhausted { failures, .. } if failures.len() == 3
    ));
    assert_eq!(hits.load(Ordering::SeqCst), 3);
    assert!(!destination.exists());
    assert_eq!(entry_count(&temporary), 0);
}

#[tokio::test]
async fn falls_back_to_the_next_candidate() {
    let server = serve(
        Router::new()
            .route("/missing", get(|| async { StatusCode::NOT_FOUND }))
            .route("/file", get(|| async { "hello" })),
    )
    .await;
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("file.txt");
    let mut request = request(server.url("/file"), destination.clone(), b"hello");
    request.sources.insert(0, server.url("/missing"));

    let status = executor()
        .download_file(&request, CancellationToken::new(), None)
        .await
        .unwrap();

    assert_eq!(status, DownloadStatus::Downloaded { bytes: 5 });
    assert_eq!(std::fs::read(destination).unwrap(), b"hello");
}

#[tokio::test]
async fn retries_server_errors_before_success() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed = hits.clone();
    let server = serve(Router::new().route(
        "/file",
        get(move || {
            let observed = observed.clone();
            async move {
                let attempt = observed.fetch_add(1, Ordering::SeqCst) + 1;
                if attempt < 3 {
                    StatusCode::INTERNAL_SERVER_ERROR.into_response()
                } else {
                    "good".into_response()
                }
            }
        }),
    ))
    .await;
    let temporary = TempDir::new().unwrap();
    let request = request(
        server.url("/file"),
        temporary.path().join("file.txt"),
        b"good",
    );

    let status = executor()
        .download_file(&request, CancellationToken::new(), None)
        .await
        .unwrap();

    assert_eq!(status, DownloadStatus::Downloaded { bytes: 4 });
    assert_eq!(hits.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn does_not_retry_not_found() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed = hits.clone();
    let server = serve(Router::new().route(
        "/missing",
        get(move || {
            let observed = observed.clone();
            async move {
                observed.fetch_add(1, Ordering::SeqCst);
                StatusCode::NOT_FOUND
            }
        }),
    ))
    .await;
    let temporary = TempDir::new().unwrap();
    let request = request(
        server.url("/missing"),
        temporary.path().join("file.txt"),
        b"good",
    );

    let error = executor()
        .download_file(&request, CancellationToken::new(), None)
        .await
        .unwrap_err();

    assert!(matches!(error, DownloadError::Exhausted { .. }));
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancellation_cleans_temporary_files() {
    let server = serve(Router::new().route(
        "/slow",
        get(|| async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            "late"
        }),
    ))
    .await;
    let temporary = TempDir::new().unwrap();
    let request = request(
        server.url("/slow"),
        temporary.path().join("file.txt"),
        b"good",
    );
    let cancellation = CancellationToken::new();
    let cancel = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.cancel();
    });

    let error = executor()
        .download_file(&request, cancellation, None)
        .await
        .unwrap_err();

    assert!(matches!(error, DownloadError::Cancelled));
    assert_eq!(entry_count(&temporary), 0);
}

#[tokio::test]
async fn batch_download_respects_concurrency_limit() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let server = serve(Router::new().route(
        "/file",
        get({
            let active = active.clone();
            let peak = peak.clone();
            move || {
                let active = active.clone();
                let peak = peak.clone();
                async move {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(current, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    "good"
                }
            }
        }),
    ))
    .await;
    let temporary = TempDir::new().unwrap();
    let requests = (0..6)
        .map(|index| {
            request(
                server.url("/file"),
                temporary.path().join(format!("file-{index}.txt")),
                b"good",
            )
        })
        .collect();

    let report = executor()
        .download_all(requests, CancellationToken::new(), None)
        .await;

    assert!(report.is_success());
    assert_eq!(report.outcomes.len(), 6);
    assert!(report.failures.is_empty());
    assert_eq!(peak.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn per_host_limit_is_enforced_independently() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let server = serve(Router::new().route(
        "/file",
        get({
            let active = active.clone();
            let peak = peak.clone();
            move || {
                let active = active.clone();
                let peak = peak.clone();
                async move {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(current, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    "good"
                }
            }
        }),
    ))
    .await;
    let temporary = TempDir::new().unwrap();
    let requests = (0..4)
        .map(|index| {
            request(
                server.url("/file"),
                temporary.path().join(format!("file-{index}.txt")),
                b"good",
            )
        })
        .collect();
    let mut config = config();
    config.max_concurrency = 4;
    config.max_concurrency_per_host = 1;
    let executor = DownloadExecutor::new(config).unwrap();

    let report = executor
        .download_all(requests, CancellationToken::new(), None)
        .await;

    assert!(report.is_success());
    assert_eq!(peak.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn batch_rejects_duplicate_destinations() {
    let server = serve(Router::new().route("/file", get(|| async { "good" }))).await;
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("file.txt");
    let first = request(server.url("/file"), destination.clone(), b"good");
    let second = request(server.url("/file"), destination, b"good");

    let report = executor()
        .download_all(vec![first, second], CancellationToken::new(), None)
        .await;

    assert!(!report.is_success());
    assert_eq!(report.outcomes.len(), 1);
    assert_eq!(report.failures.len(), 1);
    assert!(matches!(
        report.failures[0].error,
        DownloadError::ConflictingDestination(_)
    ));
}

#[tokio::test]
async fn emits_bounded_progress_events() {
    let server = serve(Router::new().route("/file", get(|| async { "good" }))).await;
    let temporary = TempDir::new().unwrap();
    let request = request(
        server.url("/file"),
        temporary.path().join("file.txt"),
        b"good",
    );
    let (progress, mut receiver) = progress_channel(2);
    let collector = tokio::spawn(async move {
        let mut events = Vec::new();
        while let Some(event) = receiver.recv().await {
            events.push(event);
        }
        events
    });

    executor()
        .download_file(&request, CancellationToken::new(), Some(progress.clone()))
        .await
        .unwrap();
    drop(progress);

    let events = collector.await.unwrap();
    assert_eq!(events.first().unwrap().stage, ProgressStage::Checking);
    assert_eq!(events.last().unwrap().stage, ProgressStage::Completed);
    assert!(
        events
            .iter()
            .any(|event| event.stage == ProgressStage::Downloading)
    );
    assert!(
        events
            .iter()
            .any(|event| event.stage == ProgressStage::Verifying)
    );
}

#[tokio::test]
async fn size_only_validation_supports_download_and_skip() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed = hits.clone();
    let server = serve(Router::new().route(
        "/file",
        get(move || {
            let observed = observed.clone();
            async move {
                observed.fetch_add(1, Ordering::SeqCst);
                "good"
            }
        }),
    ))
    .await;
    let temporary = TempDir::new().unwrap();
    let mut request = request(
        server.url("/file"),
        temporary.path().join("file.txt"),
        b"good",
    );
    request.sha1 = None;

    let executor = executor();
    assert_eq!(
        executor
            .download_file(&request, CancellationToken::new(), None)
            .await
            .unwrap(),
        DownloadStatus::Downloaded { bytes: 4 }
    );
    assert_eq!(
        executor
            .download_file(&request, CancellationToken::new(), None)
            .await
            .unwrap(),
        DownloadStatus::Skipped { bytes: 4 }
    );
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn rejects_empty_sources() {
    let temporary = TempDir::new().unwrap();
    let request = DownloadRequest {
        sources: Vec::new(),
        destination: temporary.path().join("file.txt"),
        sha1: None,
        size: None,
    };

    let error = executor()
        .download_file(&request, CancellationToken::new(), None)
        .await
        .unwrap_err();
    assert!(matches!(error, DownloadError::EmptySources));
}

#[tokio::test]
async fn failed_truncated_response_does_not_publish() {
    let server = serve(Router::new().route(
        "/truncated",
        get(|| async {
            Response::builder()
                .status(StatusCode::OK)
                .header("content-length", "100")
                .body(Body::from("good"))
                .unwrap()
        }),
    ))
    .await;
    let temporary = TempDir::new().unwrap();
    let destination = temporary.path().join("file.txt");
    let request = request(server.url("/truncated"), destination.clone(), b"good");

    let result = executor()
        .download_file(&request, CancellationToken::new(), None)
        .await;

    assert!(result.is_err());
    assert!(!destination.exists());
    assert_eq!(entry_count(&temporary), 0);
}

#[tokio::test]
async fn redacts_source_credentials_and_query_in_errors() {
    let server =
        serve(Router::new().route("/missing", get(|| async { StatusCode::NOT_FOUND }))).await;
    let temporary = TempDir::new().unwrap();
    let mut source = server.url("/missing");
    source.set_username("user").unwrap();
    source.set_password(Some("secret-password")).unwrap();
    source.set_query(Some("token=secret-value"));
    let request = request(source, temporary.path().join("file.txt"), b"good");

    let error = executor()
        .download_file(&request, CancellationToken::new(), None)
        .await
        .unwrap_err();
    let rendered = format!("{error}");

    assert!(!rendered.contains("secret-password"));
    assert!(!rendered.contains("secret-value"));
    assert!(!rendered.contains("token="));
}
