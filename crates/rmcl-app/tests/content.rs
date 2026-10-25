use std::{path::Path, time::Duration};

use axum::{Json, Router, response::IntoResponse, routing::get};
use rmcl_app::{
    AppError, ConflictPolicy, ContentGraphInstallRequest, ContentInstallPhase,
    ContentInstallRequest, ContentInstaller, ContentKind, ContentSource, IsolationMode,
    ModrinthClient, ModrinthConfig, RemoteDependency, RemoteDependencyKind, RemoteFile,
    RemoteVersion, TaskKind, TaskManager, TaskPhase, TaskProgress, TaskStatus, TaskSubject,
    VanillaInstaller, delete_instance_content, import_instance_content,
    resolve_content_install_batch, scan_instance_content, set_instance_content_enabled,
};
use rmcl_domain::Sha1Hash;
use rmcl_instance::{InstanceLayout, MinecraftLayout, VersionId};
use rmcl_net::DownloadConfig;
use serde_json::json;
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

fn sha1_text(bytes: &[u8]) -> String {
    format!("{:x}", Sha1::digest(bytes))
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

fn remote_version(
    id: &str,
    project_id: &str,
    name: &str,
    filename: &str,
    bytes: &[u8],
    url: Url,
    dependencies: Vec<RemoteDependency>,
) -> RemoteVersion {
    RemoteVersion {
        source: ContentSource::Modrinth,
        id: id.to_owned(),
        project_id: project_id.to_owned(),
        name: name.to_owned(),
        version_number: "1.0".to_owned(),
        changelog: None,
        published: None,
        version_type: "release".to_owned(),
        downloads: 0,
        game_versions: vec!["1.21.1".to_owned()],
        loaders: vec!["fabric".to_owned()],
        files: vec![RemoteFile {
            filename: filename.to_owned(),
            url,
            size: Some(bytes.len() as u64),
            sha1: Some(sha1(bytes)),
            primary: true,
        }],
        dependencies,
    }
}

fn version_json(
    id: &str,
    project_id: &str,
    name: &str,
    filename: &str,
    bytes: &[u8],
    url: Url,
    dependencies: serde_json::Value,
) -> serde_json::Value {
    json!({
        "id": id,
        "project_id": project_id,
        "name": name,
        "version_number": "1.0",
        "version_type": "release",
        "game_versions": ["1.21.1"],
        "loaders": ["fabric"],
        "files": [{
            "hashes": {"sha1": sha1_text(bytes)},
            "url": url.to_string(),
            "filename": filename,
            "primary": true,
            "size": bytes.len()
        }],
        "dependencies": dependencies
    })
}

async fn graph_server(
    primary: Vec<u8>,
    dependency: Vec<u8>,
    project_dependency: Vec<u8>,
) -> (Url, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let primary_payload = version_json(
        "primary-v",
        "primary-project",
        "Primary Mod",
        "primary.jar",
        &primary,
        base.join("files/primary.jar").unwrap(),
        json!([]),
    );
    let dependency_payload = version_json(
        "dep-v",
        "dep-project",
        "Dependency Mod",
        "dep.jar",
        &dependency,
        base.join("files/dep.jar").unwrap(),
        json!([{"project_id": "primary-project", "dependency_type": "required"}]),
    );
    let project_payload = version_json(
        "project-v",
        "project-dep",
        "Project Dependency",
        "project.jar",
        &project_dependency,
        base.join("files/project.jar").unwrap(),
        json!([]),
    );
    let router = Router::new()
        .route(
            "/v2/version/dep-v",
            get(move || {
                let payload = dependency_payload.clone();
                async move { Json(payload) }
            }),
        )
        .route(
            "/v2/project/primary-project/version",
            get(move || {
                let payload = primary_payload.clone();
                async move { Json(vec![payload]) }
            }),
        )
        .route(
            "/v2/project/project-dep/version",
            get(move || {
                let payload = project_payload.clone();
                async move { Json(vec![payload]) }
            }),
        )
        .route(
            "/files/primary.jar",
            get(move || {
                let bytes = primary.clone();
                async move { bytes }
            }),
        )
        .route(
            "/files/dep.jar",
            get(move || {
                let bytes = dependency.clone();
                async move { bytes }
            }),
        )
        .route(
            "/files/project.jar",
            get(move || {
                let bytes = project_dependency.clone();
                async move { bytes }
            }),
        );
    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (base, handle)
}

fn graph_client(base: Url) -> ModrinthClient {
    ModrinthClient::new(ModrinthConfig {
        base_url: base,
        user_agent: "RMCL-test".to_owned(),
        timeout: Duration::from_secs(5),
    })
    .unwrap()
}

fn graph_request(
    instance: InstanceLayout,
    base: &Url,
    primary: &[u8],
) -> ContentGraphInstallRequest {
    let primary = remote_version(
        "primary-v",
        "primary-project",
        "Primary Mod",
        "primary.jar",
        primary,
        base.join("files/primary.jar").unwrap(),
        vec![
            RemoteDependency {
                version_id: Some("dep-v".to_owned()),
                project_id: None,
                file_name: None,
                kind: RemoteDependencyKind::Required,
            },
            RemoteDependency {
                version_id: None,
                project_id: Some("project-dep".to_owned()),
                file_name: None,
                kind: RemoteDependencyKind::Required,
            },
            RemoteDependency {
                version_id: None,
                project_id: Some("optional-project".to_owned()),
                file_name: None,
                kind: RemoteDependencyKind::Optional,
            },
        ],
    );
    ContentGraphInstallRequest {
        instance,
        kind: ContentKind::Mod,
        version: primary,
        display_name: "Primary Mod 1.0".to_owned(),
        conflict: ConflictPolicy::Rename,
        game_version: "1.21.1".to_owned(),
        loader: Some("fabric".to_owned()),
    }
}

#[tokio::test]
async fn resolves_dependency_graph_and_installs_dependencies_first() {
    let primary = b"primary mod".to_vec();
    let dependency = b"dependency mod".to_vec();
    let project_dependency = b"project dependency".to_vec();
    let (base, server) = graph_server(
        primary.clone(),
        dependency.clone(),
        project_dependency.clone(),
    )
    .await;
    let temporary = TempDir::new().unwrap();
    let instance = instance(&temporary.path().join(".minecraft"));
    let client = graph_client(base.clone());
    let request = graph_request(instance.clone(), &base, &primary);

    let requests = resolve_content_install_batch(&client, &request, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].display_name, "Dependency Mod");
    assert_eq!(requests[1].display_name, "Project Dependency");
    assert_eq!(requests[2].display_name, "Primary Mod 1.0");

    let installer = ContentInstaller::official(DownloadConfig::default()).unwrap();
    let outcome = installer
        .install_batch(requests, CancellationToken::new(), None)
        .await
        .unwrap();
    assert_eq!(outcome.items.len(), 3);
    assert_eq!(
        std::fs::read(instance.mods_dir().join("primary.jar")).unwrap(),
        primary
    );
    assert_eq!(
        std::fs::read(instance.mods_dir().join("dep.jar")).unwrap(),
        dependency
    );
    assert_eq!(
        std::fs::read(instance.mods_dir().join("project.jar")).unwrap(),
        project_dependency
    );

    let again = resolve_content_install_batch(&client, &request, &CancellationToken::new())
        .await
        .unwrap();
    assert!(again.is_empty());
    server.abort();
}

#[tokio::test]
async fn dependency_install_task_reports_batch_progress() {
    let primary = b"primary mod".to_vec();
    let dependency = b"dependency mod".to_vec();
    let project_dependency = b"project dependency".to_vec();
    let (base, server) = graph_server(primary, dependency, project_dependency).await;
    let temporary = TempDir::new().unwrap();
    let instance = instance(&temporary.path().join(".minecraft"));
    let request = graph_request(instance.clone(), &base, b"primary mod");
    let content = ContentInstaller::official(DownloadConfig::default()).unwrap();
    let manager = TaskManager::with_content_and_modrinth(
        VanillaInstaller::official(DownloadConfig::default()).unwrap(),
        content,
        graph_client(base),
        64,
    );

    let id = manager
        .start_content_install_with_dependencies(request)
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

    assert_eq!(snapshot.status, TaskStatus::Completed);
    assert!(matches!(
        snapshot.progress,
        Some(TaskProgress::Content(progress))
            if progress.completed_files == 2 && progress.total_files == 3
    ));
    assert!(instance.mods_dir().join("primary.jar").is_file());
    assert!(instance.mods_dir().join("dep.jar").is_file());
    server.abort();
}

#[tokio::test]
async fn cancelling_dependency_resolution_marks_the_task_cancelled() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let router = Router::new().route(
        "/v2/version/slow-v",
        get(|| async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Json(json!({}))
        }),
    );
    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let temporary = TempDir::new().unwrap();
    let instance = instance(&temporary.path().join(".minecraft"));
    let primary = remote_version(
        "primary-v",
        "primary-project",
        "Primary Mod",
        "primary.jar",
        b"primary",
        base.join("files/primary.jar").unwrap(),
        vec![RemoteDependency {
            version_id: Some("slow-v".to_owned()),
            project_id: None,
            file_name: None,
            kind: RemoteDependencyKind::Required,
        }],
    );
    let request = ContentGraphInstallRequest {
        instance,
        kind: ContentKind::Mod,
        version: primary,
        display_name: "Primary Mod 1.0".to_owned(),
        conflict: ConflictPolicy::Rename,
        game_version: "1.21.1".to_owned(),
        loader: Some("fabric".to_owned()),
    };
    let manager = TaskManager::with_content_and_modrinth(
        VanillaInstaller::official(DownloadConfig::default()).unwrap(),
        ContentInstaller::official(DownloadConfig::default()).unwrap(),
        graph_client(base),
        64,
    );
    let id = manager
        .start_content_install_with_dependencies(request)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(manager.cancel(id).await);

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

    assert_eq!(snapshot.status, TaskStatus::Cancelled);
    handle.abort();
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
