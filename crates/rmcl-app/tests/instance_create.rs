use std::path::Path;

use axum::{Json, Router, routing::get};
use rmcl_app::{
    CreateInstanceRequest, InstanceCreateEvent, InstanceCreator, LoaderCatalog,
    LoaderCatalogConfig, LoaderSelection, VanillaInstaller, VanillaSources,
};
use rmcl_domain::{OperatingSystem, Platform};
use rmcl_instance::{IsolationMode, LoaderKind, MinecraftLayout, VersionId};
use rmcl_net::DownloadConfig;
use serde_json::json;
use sha1::{Digest, Sha1};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use url::Url;

fn sha1(bytes: &[u8]) -> String {
    format!("{:x}", Sha1::digest(bytes))
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn platform() -> Platform {
    Platform::new(OperatingSystem::Linux, "6.1", "x86_64")
}

async fn loader_server() -> (Url, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let library = b"loader-library".to_vec();
    let library_sha1 = sha1(&library);
    let library_size = library.len();
    let library_url = base.join("repository/").unwrap().to_string();
    let profile = json!({
        "id": "fabric-loader-0.16.10-1.21.1",
        "inheritsFrom": "1.21.1",
        "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
        "arguments": {"game": [], "jvm": ["-DFabric=true"]},
        "libraries": [{
            "name": "example:loader:1.0",
            "url": library_url,
            "sha1": library_sha1,
            "size": library_size
        }]
    });
    let router = Router::new()
        .route(
            "/v2/versions/loader/1.21.1/0.16.10/profile/json",
            get(move || {
                let profile = profile.clone();
                async move { Json(profile) }
            }),
        )
        .route(
            "/repository/example/loader/1.0/loader-1.0.jar",
            get(move || {
                let library = library.clone();
                async move { library }
            }),
        );
    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (base, handle)
}

fn write_base(root: &Path) {
    let client = b"client";
    let index = b"{}";
    let layout = MinecraftLayout::new(root);
    write(
        &layout.version_json_path(&VersionId::new("1.21.1").unwrap()),
        serde_json::to_vec(&json!({
            "id": "1.21.1",
            "mainClass": "net.minecraft.client.main.Main",
            "type": "release",
            "arguments": {"game": ["--username", "${auth_player_name}"], "jvm": ["-cp", "${classpath}"]},
            "downloads": {
                "client": {
                    "url": "https://example.invalid/client.jar",
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
        .unwrap()
        .as_slice(),
    );
    write(
        &layout.client_jar_path(&VersionId::new("1.21.1").unwrap()),
        client,
    );
    write(&layout.asset_index_path("test-index").unwrap(), index);
}

fn installer() -> VanillaInstaller {
    VanillaInstaller::new(
        DownloadConfig::default(),
        VanillaSources {
            manifest_url: Url::parse("https://example.invalid/manifest.json").unwrap(),
            asset_object_base: Url::parse("https://example.invalid/objects/").unwrap(),
        },
    )
    .unwrap()
}

fn request(
    root: &Path,
    instance_id: &str,
    loader: Option<LoaderSelection>,
) -> CreateInstanceRequest {
    CreateInstanceRequest {
        minecraft_root: root.to_path_buf(),
        instance_id: instance_id.to_owned(),
        name: "Test Instance".to_owned(),
        base_version_id: "1.21.1".to_owned(),
        platform: platform(),
        loader,
    }
}

#[tokio::test]
async fn creates_isolated_fabric_instance_from_existing_base() {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path().join(".minecraft");
    write_base(&root);
    let (loader_base, server) = loader_server().await;
    let catalog = LoaderCatalog::new(LoaderCatalogConfig {
        fabric_base: loader_base.clone(),
        quilt_base: loader_base,
        user_agent: "RMCL-test".to_owned(),
        timeout: std::time::Duration::from_secs(5),
    })
    .unwrap();
    let creator = InstanceCreator::new(installer(), catalog);
    let (sender, mut receiver) = tokio::sync::mpsc::channel(512);

    let outcome = creator
        .create(
            request(
                &root,
                "fabric-test",
                Some(LoaderSelection {
                    kind: LoaderKind::Fabric,
                    version: "0.16.10".to_owned(),
                }),
            ),
            CancellationToken::new(),
            Some(sender),
        )
        .await
        .unwrap();

    let mut phases = Vec::new();
    while let Ok(event) = receiver.try_recv() {
        if let InstanceCreateEvent::Phase(phase) = event {
            phases.push(phase);
        }
    }
    assert!(phases.contains(&rmcl_app::InstanceCreatePhase::FetchingLoader));
    assert!(phases.contains(&rmcl_app::InstanceCreatePhase::WritingInstance));
    assert_eq!(
        phases.last(),
        Some(&rmcl_app::InstanceCreatePhase::Completed)
    );

    let instance = outcome.install.instance;
    assert_eq!(instance.isolation(), IsolationMode::Isolated);
    assert_eq!(instance.root(), root.join("versions/fabric-test"));
    assert_eq!(
        outcome.install.version.main_class.as_deref(),
        Some("net.fabricmc.loader.impl.launch.knot.KnotClient")
    );
    let manifest = instance.load_manifest().unwrap().unwrap();
    assert_eq!(manifest.minecraft_version, "1.21.1");
    assert_eq!(
        manifest.loader.as_ref().map(|loader| loader.kind),
        Some(LoaderKind::Fabric)
    );
    let json = serde_json::from_slice::<serde_json::Value>(
        &std::fs::read(instance.version_json_path()).unwrap(),
    )
    .unwrap();
    assert_eq!(json["id"], "fabric-test");
    assert_eq!(json["inheritsFrom"], "1.21.1");
    assert!(root.join("versions/1.21.1/1.21.1.jar").is_file());
    assert_eq!(
        std::fs::read(root.join("libraries/example/loader/1.0/loader-1.0.jar")).unwrap(),
        b"loader-library"
    );
    server.abort();
}

#[tokio::test]
async fn instance_create_task_exposes_subject_and_completes() {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path().join(".minecraft");
    write_base(&root);
    let (loader_base, server) = loader_server().await;
    let catalog = LoaderCatalog::new(LoaderCatalogConfig {
        fabric_base: loader_base.clone(),
        quilt_base: loader_base,
        user_agent: "RMCL-test".to_owned(),
        timeout: std::time::Duration::from_secs(5),
    })
    .unwrap();
    let creator = InstanceCreator::new(installer(), catalog);
    let manager = rmcl_app::TaskManager::with_creator_only(installer(), creator, 64);

    let id = manager
        .start_create_instance(request(
            &root,
            "fabric-task",
            Some(LoaderSelection {
                kind: LoaderKind::Fabric,
                version: "0.16.10".to_owned(),
            }),
        ))
        .await
        .unwrap();

    let snapshot = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let snapshot = manager.snapshot(id).await.unwrap();
            if snapshot.status.is_terminal() {
                break snapshot;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    assert_eq!(snapshot.kind, rmcl_app::TaskKind::InstanceCreate);
    assert_eq!(snapshot.status, rmcl_app::TaskStatus::Completed);
    assert_eq!(
        snapshot.phase,
        Some(rmcl_app::TaskPhase::InstanceCreate(
            rmcl_app::InstanceCreatePhase::Completed
        ))
    );
    assert!(matches!(
        snapshot.subject,
        Some(rmcl_app::TaskSubject::Instance { ref id, .. }) if id == "fabric-task"
    ));
    server.abort();
}

#[tokio::test]
async fn creates_custom_vanilla_instance_without_a_loader() {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path().join(".minecraft");
    write_base(&root);
    let catalog = LoaderCatalog::new(LoaderCatalogConfig {
        fabric_base: Url::parse("https://example.invalid/").unwrap(),
        quilt_base: Url::parse("https://example.invalid/").unwrap(),
        user_agent: "RMCL-test".to_owned(),
        timeout: std::time::Duration::from_secs(5),
    })
    .unwrap();
    let creator = InstanceCreator::new(installer(), catalog);

    let outcome = creator
        .create(
            request(&root, "custom-vanilla", None),
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap();

    let instance = outcome.install.instance;
    assert_eq!(instance.isolation(), IsolationMode::Isolated);
    assert_eq!(
        outcome.install.version.main_class.as_deref(),
        Some("net.minecraft.client.main.Main")
    );
    let manifest = instance.load_manifest().unwrap().unwrap();
    assert!(manifest.loader.is_none());
    assert_eq!(manifest.minecraft_version, "1.21.1");
}

#[tokio::test]
async fn refuses_to_overwrite_an_existing_instance_directory() {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path().join(".minecraft");
    write_base(&root);
    std::fs::create_dir_all(root.join("versions/custom-vanilla")).unwrap();
    let catalog = LoaderCatalog::new(LoaderCatalogConfig {
        fabric_base: Url::parse("https://example.invalid/").unwrap(),
        quilt_base: Url::parse("https://example.invalid/").unwrap(),
        user_agent: "RMCL-test".to_owned(),
        timeout: std::time::Duration::from_secs(5),
    })
    .unwrap();
    let creator = InstanceCreator::new(installer(), catalog);

    let error = creator
        .create(
            request(&root, "custom-vanilla", None),
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap_err();

    assert!(matches!(error, rmcl_app::AppError::InvalidRequest(_)));
}
