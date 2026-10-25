use std::{path::Path, time::Duration};

use axum::{Router, routing::get};
use rmcl_app::{
    AppError, InstallPhase, TaskEvent, TaskId, TaskManager, TaskPhase, TaskStatus,
    VanillaInstallRequest, VanillaInstaller, VanillaSources, VersionSource,
};
use rmcl_domain::{OperatingSystem, Platform};
use rmcl_net::DownloadConfig;
use serde_json::json;
use sha1::{Digest, Sha1};
use tempfile::TempDir;
use tokio::sync::broadcast;
use url::Url;

fn installer() -> VanillaInstaller {
    VanillaInstaller::new(
        DownloadConfig {
            max_concurrency: 2,
            max_concurrency_per_host: 2,
            max_attempts_per_source: 1,
            connect_timeout: Duration::from_secs(1),
            request_timeout: Duration::from_secs(2),
            initial_retry_delay: Duration::from_millis(1),
            max_retry_delay: Duration::from_millis(5),
        },
        VanillaSources {
            manifest_url: Url::parse("https://example.invalid/manifest.json").unwrap(),
            asset_object_base: Url::parse("https://example.invalid/objects/").unwrap(),
        },
    )
    .unwrap()
}

fn request(root: &Path) -> VanillaInstallRequest {
    VanillaInstallRequest {
        minecraft_root: root.to_path_buf(),
        version_id: "test".to_owned(),
        platform: Platform::new(OperatingSystem::Linux, "6.1", "x86_64"),
        source: VersionSource::Existing,
    }
}

fn write_metadata(root: &Path, client_url: Url, include_client: bool) {
    let client = b"client";
    let index = b"{}";
    let version = serde_json::to_vec(&json!({
        "id": "test",
        "mainClass": "example.Main",
        "arguments": {"jvm": ["-cp", "${classpath}"], "game": []},
        "downloads": {
            "client": {
                "url": client_url.to_string(),
                "sha1": sha1(client),
                "size": client.len()
            }
        },
        "assetIndex": {
            "id": "test-index",
            "url": "https://example.invalid/index.json",
            "sha1": sha1(index),
            "size": index.len()
        },
        "libraries": []
    }))
    .unwrap();
    std::fs::create_dir_all(root.join("versions/test")).unwrap();
    std::fs::create_dir_all(root.join("assets/indexes")).unwrap();
    std::fs::write(root.join("versions/test/test.json"), version).unwrap();
    std::fs::write(root.join("assets/indexes/test-index.json"), index).unwrap();
    if include_client {
        std::fs::write(root.join("versions/test/test.jar"), client).unwrap();
    }
}

fn seeded() -> (TempDir, VanillaInstallRequest, TaskManager) {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path().join(".minecraft");
    write_metadata(
        &root,
        Url::parse("https://example.invalid/client.jar").unwrap(),
        true,
    );
    let request = request(&root);
    let manager = TaskManager::new(installer(), 64);
    (temporary, request, manager)
}

async fn wait_for_status(
    receiver: &mut broadcast::Receiver<TaskEvent>,
    id: TaskId,
    status: TaskStatus,
) -> rmcl_app::TaskSnapshot {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match receiver.recv().await {
                Ok(TaskEvent::Snapshot(snapshot))
                    if snapshot.id == id && snapshot.status == status =>
                {
                    return snapshot;
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => {
                    panic!("task event channel closed")
                }
            }
        }
    })
    .await
    .unwrap()
}

async fn wait_for_phase(
    receiver: &mut broadcast::Receiver<TaskEvent>,
    id: TaskId,
    phase: InstallPhase,
) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match receiver.recv().await {
                Ok(TaskEvent::Snapshot(snapshot))
                    if snapshot.id == id && snapshot.phase == Some(TaskPhase::Install(phase)) =>
                {
                    return;
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => {
                    panic!("task event channel closed")
                }
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn terminal_snapshots_are_retained_and_ids_are_not_reused() {
    let (_temporary, request, manager) = seeded();
    let mut events = manager.subscribe();

    let first = manager.start_install(request.clone()).await.unwrap();
    let completed = wait_for_status(&mut events, first, TaskStatus::Completed).await;
    assert_eq!(completed.kind, rmcl_app::TaskKind::VanillaInstall);
    assert_eq!(
        completed.phase,
        Some(TaskPhase::Install(InstallPhase::Completed))
    );
    assert!(completed.error.is_none());
    assert_eq!(manager.snapshot(first).await, Some(completed));

    let second = manager.start_install(request).await.unwrap();
    assert!(second > first);
    wait_for_status(&mut events, second, TaskStatus::Completed).await;
    assert!(manager.dismiss(first).await);
    assert!(manager.snapshot(first).await.is_none());
    assert!(manager.dismiss(second).await);
    assert!(!manager.dismiss(second).await);
}

struct DelayedFixture {
    _temporary: TempDir,
    root: std::path::PathBuf,
    request: VanillaInstallRequest,
    manager: TaskManager,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for DelayedFixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn delayed() -> DelayedFixture {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let base = Url::parse(&format!("http://{address}/")).unwrap();
    let router = Router::new().route(
        "/client.jar",
        get(|| async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            b"client".to_vec()
        }),
    );
    let server = tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            eprintln!("test server failed: {error}");
        }
    });
    let temporary = TempDir::new().unwrap();
    let root = temporary.path().join(".minecraft");
    write_metadata(&root, base.join("client.jar").unwrap(), false);
    let request = request(&root);
    let manager = TaskManager::new(installer(), 64);
    DelayedFixture {
        _temporary: temporary,
        root,
        request,
        manager,
        server,
    }
}

#[tokio::test]
async fn duplicate_install_is_rejected_and_cancellation_has_two_states() {
    let fixture = delayed().await;
    let manager = fixture.manager.clone();
    let mut events = manager.subscribe();
    let first = manager
        .start_install(fixture.request.clone())
        .await
        .unwrap();
    wait_for_phase(&mut events, first, InstallPhase::DownloadingBase).await;

    let duplicate = manager.start_install(fixture.request.clone()).await;
    assert!(matches!(
        duplicate,
        Err(AppError::TaskAlreadyRunning { existing }) if existing == first
    ));
    assert!(manager.cancel(first).await);
    wait_for_status(&mut events, first, TaskStatus::Cancelling).await;
    let cancelled = wait_for_status(&mut events, first, TaskStatus::Cancelled).await;
    assert!(cancelled.error.is_none());
    assert!(!manager.cancel(first).await);

    std::fs::write(fixture.root.join("versions/test/test.jar"), b"client").unwrap();
    let second = manager
        .start_install(fixture.request.clone())
        .await
        .unwrap();
    assert!(second > first);
    wait_for_status(&mut events, second, TaskStatus::Completed).await;
}

fn sha1(bytes: &[u8]) -> String {
    hex::encode(Sha1::digest(bytes))
}
