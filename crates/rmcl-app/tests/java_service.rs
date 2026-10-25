use rmcl_app::{AppError, JavaEvent, JavaService, JavaServiceConfig};
use rmcl_domain::{Architecture, OperatingSystem};
use rmcl_java::JavaRequirement;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[cfg(unix)]
use axum::{Router, routing::get};
#[cfg(unix)]
use rmcl_app::JavaSelectionOrigin;
#[cfg(unix)]
use rmcl_java::UrlPolicy;
#[cfg(unix)]
use serde_json::json;
#[cfg(unix)]
use sha1::{Digest, Sha1};
#[cfg(unix)]
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
#[cfg(unix)]
use url::Url;

fn requirement() -> JavaRequirement {
    JavaRequirement {
        component: None,
        major_version: Some(21),
        maximum_major: None,
        operating_system: OperatingSystem::Linux,
        architecture: Architecture::X86_64,
    }
}

fn config(root: &std::path::Path) -> JavaServiceConfig {
    let mut config = JavaServiceConfig::new(root.join("store"));
    config.include_java_home = false;
    config.include_path = false;
    config.auto_install = false;
    config
}

fn drain(receiver: &mut mpsc::Receiver<JavaEvent>) -> Vec<JavaEvent> {
    let mut events = Vec::new();
    while let Ok(event) = receiver.try_recv() {
        events.push(event);
    }
    events
}

#[tokio::test]
async fn missing_java_without_automatic_install_is_structured() {
    let temporary = TempDir::new().unwrap();
    let service = JavaService::new(config(temporary.path())).unwrap();
    let (sender, mut receiver) = mpsc::channel(8);

    let result = service
        .ensure(&requirement(), CancellationToken::new(), Some(sender))
        .await;

    assert!(matches!(result, Err(AppError::JavaNotFound { .. })));
    assert_eq!(drain(&mut receiver), [JavaEvent::Checking]);
}

#[cfg(unix)]
fn write_fake_java(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        b"#!/bin/sh\nprintf 'java.version = 21.0.7\\nos.name = Linux\\nos.arch = amd64\\njava.vendor = TestVendor\\n' >&2\n",
    )
    .unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn custom_java_path_is_probed_without_automatic_discovery() {
    let temporary = TempDir::new().unwrap();
    let path = temporary.path().join("custom/bin/java");
    write_fake_java(&path);
    let service = JavaService::new(config(temporary.path())).unwrap();

    let runtime = service.probe(path.clone()).await.unwrap();

    assert_eq!(runtime.executable, path);
    assert_eq!(runtime.major_version, 21);
    assert_eq!(runtime.source, rmcl_java::JavaRuntimeSource::Custom);
}

#[cfg(unix)]
#[tokio::test]
async fn detected_compatible_java_does_not_install() {
    let temporary = TempDir::new().unwrap();
    let managed = temporary.path().join("managed");
    write_fake_java(&managed.join("bin/java"));
    let mut config = config(temporary.path());
    config.managed_roots.push(managed);
    let service = JavaService::new(config).unwrap();
    let (sender, mut receiver) = mpsc::channel(8);

    let selection = service
        .ensure(&requirement(), CancellationToken::new(), Some(sender))
        .await
        .unwrap();

    assert_eq!(selection.origin, JavaSelectionOrigin::Detected);
    assert_eq!(selection.runtime.major_version, 21);
    assert_eq!(drain(&mut receiver), [JavaEvent::Checking]);
}

#[cfg(unix)]
struct RuntimeServer {
    base: Url,
    index_hits: Arc<AtomicUsize>,
    manifest_hits: Arc<AtomicUsize>,
    java_hits: Arc<AtomicUsize>,
    handle: tokio::task::JoinHandle<()>,
}

#[cfg(unix)]
impl Drop for RuntimeServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

#[cfg(unix)]
async fn runtime_server() -> RuntimeServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let java = b"#!/bin/sh\nprintf 'java.version = 21.0.7\\nos.name = Linux\\nos.arch = amd64\\njava.vendor = TestVendor\\n' >&2\n".to_vec();
    let manifest = serde_json::to_vec(&json!({
        "files": {
            "bin/java": {
                "type": "file",
                "executable": true,
                "downloads": {
                    "raw": {
                        "url": base.join("bin/java").unwrap().to_string(),
                        "sha1": sha1(&java),
                        "size": java.len()
                    }
                }
            }
        }
    }))
    .unwrap();
    let index = serde_json::to_vec(&json!({
        "linux": {
            "java-runtime-delta": [{
                "version": {"name": "21.0.7", "released": "2026-01-01T00:00:00+00:00"},
                "manifest": {
                    "url": base.join("manifest.json").unwrap().to_string(),
                    "sha1": sha1(&manifest),
                    "size": manifest.len()
                }
            }]
        }
    }))
    .unwrap();
    let index_hits = Arc::new(AtomicUsize::new(0));
    let manifest_hits = Arc::new(AtomicUsize::new(0));
    let java_hits = Arc::new(AtomicUsize::new(0));
    let router = Router::new()
        .route(
            "/index.json",
            get({
                let hits = index_hits.clone();
                move || {
                    let hits = hits.clone();
                    let index = index.clone();
                    async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        index
                    }
                }
            }),
        )
        .route(
            "/manifest.json",
            get({
                let hits = manifest_hits.clone();
                move || {
                    let hits = hits.clone();
                    let manifest = manifest.clone();
                    async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        manifest
                    }
                }
            }),
        )
        .route(
            "/bin/java",
            get({
                let hits = java_hits.clone();
                move || {
                    let hits = hits.clone();
                    let java = java.clone();
                    async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        java
                    }
                }
            }),
        );
    let handle = tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            eprintln!("test runtime server failed: {error}");
        }
    });
    RuntimeServer {
        base,
        index_hits,
        manifest_hits,
        java_hits,
        handle,
    }
}

#[cfg(unix)]
#[tokio::test]
async fn automatic_install_is_detected_and_reused_without_network() {
    let server = runtime_server().await;
    let temporary = TempDir::new().unwrap();
    let mut config = config(temporary.path());
    config.auto_install = true;
    config.runtime_index_url = Some(server.base.join("index.json").unwrap());
    config.url_policy = UrlPolicy::AnyHttp;
    let service = JavaService::new(config).unwrap();
    let (sender, mut receiver) = mpsc::channel(8);

    let installed = service
        .ensure(&requirement(), CancellationToken::new(), Some(sender))
        .await
        .unwrap();

    assert_eq!(installed.origin, JavaSelectionOrigin::Installed);
    assert!(
        installed
            .runtime
            .executable
            .ends_with("linux/java-runtime-delta/bin/java")
    );
    assert!(
        temporary
            .path()
            .join("store/linux/java-runtime-delta.json")
            .is_file()
    );
    assert_eq!(
        drain(&mut receiver),
        [JavaEvent::Checking, JavaEvent::Installing]
    );

    let (sender, mut receiver) = mpsc::channel(8);
    let detected = service
        .ensure(&requirement(), CancellationToken::new(), Some(sender))
        .await
        .unwrap();
    assert_eq!(detected.origin, JavaSelectionOrigin::Detected);
    assert_eq!(
        std::fs::canonicalize(&detected.runtime.executable).unwrap(),
        std::fs::canonicalize(&installed.runtime.executable).unwrap()
    );
    assert_eq!(drain(&mut receiver), [JavaEvent::Checking]);
    assert_eq!(server.index_hits.load(Ordering::SeqCst), 1);
    assert_eq!(server.manifest_hits.load(Ordering::SeqCst), 1);
    assert_eq!(server.java_hits.load(Ordering::SeqCst), 1);
}

#[cfg(unix)]
fn sha1(bytes: &[u8]) -> String {
    hex::encode(Sha1::digest(bytes))
}
