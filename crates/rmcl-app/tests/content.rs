use std::{path::Path, time::Duration};

use axum::{Router, response::IntoResponse, routing::get};
use rmcl_app::{
    AppError, ConflictPolicy, ContentInstallPhase, ContentInstallRequest, ContentInstaller,
    ContentKind, IsolationMode, RemoteFile, TaskKind, TaskManager, TaskPhase, TaskStatus,
    TaskSubject, VanillaInstaller, delete_instance_content, import_instance_content,
    scan_instance_content, set_instance_content_enabled,
};
use rmcl_domain::Sha1Hash;
use rmcl_instance::{InstanceLayout, MinecraftLayout, VersionId};
use rmcl_net::DownloadConfig;
use sha1::{Digest, Sha1};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use url::Url;

fn instance(root: &Path) -> InstanceLayout {
    let instance = MinecraftLayout::new(root)
        .instance(VersionId::new("1.21.1").unwrap(), IsolationMode::Isolated);
    instance.ensure_content_dirs().unwrap();
    instance
}

fn write(path: &Path, contents: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn sha1(bytes: &[u8]) -> Sha1Hash {
    format!("{:x}", Sha1::digest(bytes)).parse().unwrap()
}

async fn mod_server(bytes: Vec<u8>) -> (Url, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let router = Router::new().route(
        "/mod.jar",
        get(move || {
            let bytes = bytes.clone();
            async move { bytes.into_response() }
        }),
    );
    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (base, handle)
}

fn remote_file(base: &Url, bytes: &[u8], sha1: Option<Sha1Hash>) -> RemoteFile {
    RemoteFile {
        filename: "example.jar".to_owned(),
        url: base.join("mod.jar").unwrap(),
        size: Some(bytes.len() as u64),
        sha1: Some(sha1.unwrap_or_else(|| self::sha1(bytes))),
        primary: true,
    }
}

#[tokio::test]
async fn imports_lists_toggles_and_deletes_instance_content() {
    let temporary = TempDir::new().unwrap();
    let instance = instance(&temporary.path().join(".minecraft"));
    let source = temporary.path().join("downloads/example.jar");
    write(&source, b"mod");

    let imported = import_instance_content(
        instance.clone(),
        ContentKind::Mod,
        source,
        ConflictPolicy::Rename,
    )
    .await
    .unwrap();

    assert_eq!(imported.relative_path.as_str(), "example.jar");
    assert!(imported.enabled);
    let library = scan_instance_content(instance.clone()).await.unwrap();
    assert_eq!(library.count(ContentKind::Mod), 1);

    let disabled = set_instance_content_enabled(
        instance.clone(),
        ContentKind::Mod,
        imported.relative_path,
        false,
    )
    .await
    .unwrap();
    assert!(!disabled.enabled);
    assert_eq!(disabled.relative_path.as_str(), "example.jar.disabled");

    let library = scan_instance_content(instance.clone()).await.unwrap();
    assert_eq!(library.count(ContentKind::Mod), 1);
    assert!(!library.items[0].enabled);

    delete_instance_content(instance.clone(), ContentKind::Mod, disabled.relative_path)
        .await
        .unwrap();
    assert!(
        scan_instance_content(instance)
            .await
            .unwrap()
            .items
            .is_empty()
    );
}

#[tokio::test]
async fn downloads_verifies_and_imports_remote_content() {
    let bytes = b"remote mod".to_vec();
    let (base, server) = mod_server(bytes.clone()).await;
    let temporary = TempDir::new().unwrap();
    let instance = instance(&temporary.path().join(".minecraft"));
    let installer = ContentInstaller::official(DownloadConfig::default()).unwrap();

    let outcome = installer
        .install(
            ContentInstallRequest {
                instance: instance.clone(),
                kind: ContentKind::Mod,
                file: remote_file(&base, &bytes, None),
                display_name: "Example".to_owned(),
                conflict: ConflictPolicy::Rename,
            },
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap();

    assert_eq!(outcome.content.relative_path.as_str(), "example.jar");
    assert_eq!(outcome.downloaded_bytes, bytes.len() as u64);
    assert!(!outcome.skipped);
    assert_eq!(
        std::fs::read(instance.mods_dir().join("example.jar")).unwrap(),
        bytes
    );

    let second = installer
        .install(
            ContentInstallRequest {
                instance: instance.clone(),
                kind: ContentKind::Mod,
                file: remote_file(&base, &bytes, None),
                display_name: "Example".to_owned(),
                conflict: ConflictPolicy::Rename,
            },
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(second.content.relative_path.as_str(), "example (1).jar");
    server.abort();
}

#[tokio::test]
async fn remote_hash_mismatch_does_not_publish_content() {
    let bytes = b"remote mod".to_vec();
    let (base, server) = mod_server(bytes.clone()).await;
    let temporary = TempDir::new().unwrap();
    let instance = instance(&temporary.path().join(".minecraft"));
    let installer = ContentInstaller::official(DownloadConfig::default()).unwrap();
    let bad_hash = "0000000000000000000000000000000000000000".parse().unwrap();

    let error = installer
        .install(
            ContentInstallRequest {
                instance: instance.clone(),
                kind: ContentKind::Mod,
                file: remote_file(&base, &bytes, Some(bad_hash)),
                display_name: "Example".to_owned(),
                conflict: ConflictPolicy::Rename,
            },
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap_err();

    assert!(matches!(error, AppError::ContentDownload { .. }));
    assert!(
        scan_instance_content(instance)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    server.abort();
}

#[tokio::test]
async fn remote_install_rejects_unsafe_names_and_missing_hashes() {
    let temporary = TempDir::new().unwrap();
    let instance = instance(&temporary.path().join(".minecraft"));
    let installer = ContentInstaller::official(DownloadConfig::default()).unwrap();
    let base = Url::parse("https://example.invalid/").unwrap();

    let unsafe_name = installer
        .install(
            ContentInstallRequest {
                instance: instance.clone(),
                kind: ContentKind::Mod,
                file: RemoteFile {
                    filename: "../escape.jar".to_owned(),
                    url: base.join("mod.jar").unwrap(),
                    size: Some(1),
                    sha1: Some(sha1(b"x")),
                    primary: true,
                },
                display_name: "Unsafe".to_owned(),
                conflict: ConflictPolicy::Rename,
            },
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(unsafe_name, AppError::InvalidRequest(_)));

    let missing_hash = installer
        .install(
            ContentInstallRequest {
                instance,
                kind: ContentKind::Mod,
                file: RemoteFile {
                    filename: "safe.jar".to_owned(),
                    url: base.join("mod.jar").unwrap(),
                    size: Some(1),
                    sha1: None,
                    primary: true,
                },
                display_name: "No hash".to_owned(),
                conflict: ConflictPolicy::Rename,
            },
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(missing_hash, AppError::InvalidRequest(_)));
}

#[tokio::test]
async fn content_task_exposes_subject_and_completes() {
    let bytes = b"remote mod".to_vec();
    let (base, server) = mod_server(bytes.clone()).await;
    let temporary = TempDir::new().unwrap();
    let instance = instance(&temporary.path().join(".minecraft"));
    let content = ContentInstaller::official(DownloadConfig::default()).unwrap();
    let manager = TaskManager::with_content_only(
        VanillaInstaller::official(DownloadConfig::default()).unwrap(),
        content,
        64,
    );

    let id = manager
        .start_content_install(ContentInstallRequest {
            instance: instance.clone(),
            kind: ContentKind::Mod,
            file: remote_file(&base, &bytes, None),
            display_name: "Example Mod".to_owned(),
            conflict: ConflictPolicy::Rename,
        })
        .await
        .unwrap();

    let snapshot = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = manager.snapshot(id).await.unwrap();
            if snapshot.status.is_terminal() {
                break snapshot;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    assert_eq!(snapshot.kind, TaskKind::ContentInstall);
    assert_eq!(snapshot.status, TaskStatus::Completed);
    assert_eq!(
        snapshot.phase,
        Some(TaskPhase::Content(ContentInstallPhase::Completed))
    );
    assert!(matches!(
        snapshot.subject,
        Some(TaskSubject::Content { ref name, .. }) if name == "Example Mod"
    ));
    server.abort();
}
