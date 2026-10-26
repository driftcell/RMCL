#[cfg(unix)]
use std::io::Write;
use std::path::{Path, PathBuf};

use axum::{Json, Router, routing::get};
use rmcl_app::{
    CreateInstanceRequest, InstanceCreateEvent, InstanceCreator, LoaderCatalog,
    LoaderCatalogConfig, LoaderSelection, VanillaInstaller, VanillaSources,
};
#[cfg(unix)]
use rmcl_app::{ForgeInstaller, JavaService, JavaServiceConfig};
use rmcl_domain::{OperatingSystem, Platform};
use rmcl_instance::{InstanceManifest, IsolationMode, LoaderKind, MinecraftLayout, VersionId};
use rmcl_net::DownloadConfig;
use serde_json::json;
use sha1::{Digest, Sha1};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use url::Url;
#[cfg(unix)]
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

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
        copy_content_from: None,
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
        ..LoaderCatalogConfig::official().unwrap()
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
                    installer_version: "0.16.10".to_owned(),
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
async fn copies_existing_instance_content_when_creating_loader_instance() {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path().join(".minecraft");
    write_base(&root);

    let layout = MinecraftLayout::new(&root);
    let source_id = VersionId::new("1.21.1").unwrap();
    let source = layout.instance(source_id.clone(), IsolationMode::Isolated);
    source.ensure_content_dirs().unwrap();
    let mut source_manifest = InstanceManifest::new(&source_id, IsolationMode::Isolated);
    source_manifest.name = "Source".to_owned();
    source_manifest.minecraft_version = "1.21.1".to_owned();
    source_manifest.java = Some(PathBuf::from("/opt/java/bin/java"));
    source_manifest.max_memory_mb = Some(4096);
    source_manifest.window_width = Some(1280);
    source_manifest.window_height = Some(720);
    source.write_manifest(&source_manifest).unwrap();
    write(&source.mods_dir().join("example.jar"), b"mod");
    write(&source.game_dir().join("options.txt"), b"options");
    write(&source.game_dir().join("config/example.toml"), b"config");
    write(&source.game_dir().join("saves/world/level.dat"), b"save");
    write(&source.game_dir().join("logs/latest.log"), b"log");

    let (loader_base, server) = loader_server().await;
    let catalog = LoaderCatalog::new(LoaderCatalogConfig {
        fabric_base: loader_base.clone(),
        quilt_base: loader_base,
        user_agent: "RMCL-test".to_owned(),
        timeout: std::time::Duration::from_secs(5),
        ..LoaderCatalogConfig::official().unwrap()
    })
    .unwrap();
    let creator = InstanceCreator::new(installer(), catalog);
    let (sender, mut receiver) = tokio::sync::mpsc::channel(512);
    let mut request = request(
        &root,
        "fabric-copy",
        Some(LoaderSelection {
            kind: LoaderKind::Fabric,
            version: "0.16.10".to_owned(),
            installer_version: "0.16.10".to_owned(),
        }),
    );
    request.copy_content_from = Some("1.21.1".to_owned());

    let outcome = creator
        .create(request, CancellationToken::new(), Some(sender))
        .await
        .unwrap();

    let mut phases = Vec::new();
    while let Ok(event) = receiver.try_recv() {
        if let InstanceCreateEvent::Phase(phase) = event {
            phases.push(phase);
        }
    }
    assert!(phases.contains(&rmcl_app::InstanceCreatePhase::CopyingContent));

    let target = outcome.install.instance;
    assert_eq!(
        std::fs::read(target.mods_dir().join("example.jar")).unwrap(),
        b"mod"
    );
    assert_eq!(
        std::fs::read(target.game_dir().join("options.txt")).unwrap(),
        b"options"
    );
    assert_eq!(
        std::fs::read(target.game_dir().join("config/example.toml")).unwrap(),
        b"config"
    );
    assert_eq!(
        std::fs::read(target.game_dir().join("saves/world/level.dat")).unwrap(),
        b"save"
    );
    assert!(!target.game_dir().join("logs/latest.log").exists());
    let target_manifest = target.load_manifest().unwrap().unwrap();
    assert_eq!(
        target_manifest.loader.as_ref().map(|loader| loader.kind),
        Some(LoaderKind::Fabric)
    );
    assert_eq!(target_manifest.name, "Test Instance");
    assert_eq!(
        target_manifest.java.as_deref(),
        Some(Path::new("/opt/java/bin/java"))
    );
    assert_eq!(target_manifest.max_memory_mb, Some(4096));
    assert_eq!(target_manifest.window_width, Some(1280));
    assert_eq!(target_manifest.window_height, Some(720));

    assert_eq!(
        std::fs::read(source.mods_dir().join("example.jar")).unwrap(),
        b"mod"
    );
    assert_eq!(
        std::fs::read(source.game_dir().join("options.txt")).unwrap(),
        b"options"
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
        ..LoaderCatalogConfig::official().unwrap()
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
                installer_version: "0.16.10".to_owned(),
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

#[cfg(unix)]
fn forge_installer_jar(path: &Path) {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("install_profile.json", options).unwrap();
    zip.write_all(
        json!({
            "spec": 1,
            "minecraft": "1.21.1",
            "json": "/version.json",
            "libraries": [],
            "processors": [],
            "data": {}
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    zip.start_file("version.json", options).unwrap();
    zip.write_all(
        json!({
            "id": "1.21.1-forge-test",
            "inheritsFrom": "1.21.1",
            "mainClass": "example.ForgeMain",
            "libraries": []
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    zip.finish().unwrap();
}

#[cfg(unix)]
fn write_fake_java(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        "#!/bin/sh\nif [ \"$1\" = \"-XshowSettings:properties\" ]; then\n  printf 'java.version = 21.0.7\\nos.name = Linux\\nos.arch = amd64\\njava.vendor = TestVendor\\n' >&2\n  exit 0\nfi\nexit 0\n",
    )
    .unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
}

#[cfg(unix)]
async fn forge_installer_server() -> (Url, tokio::task::JoinHandle<()>) {
    let temporary = TempDir::new().unwrap();
    let installer_path = temporary.path().join("installer.jar");
    forge_installer_jar(&installer_path);
    let installer = std::fs::read(&installer_path).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let router = Router::new().route(
        "/net/minecraftforge/forge/1.21.1-52.1.20/forge-1.21.1-52.1.20-installer.jar",
        get(move || {
            let installer = installer.clone();
            async move { installer }
        }),
    );
    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (base, handle)
}

#[cfg(unix)]
#[tokio::test]
async fn creates_isolated_forge_instance_through_installer() {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path().join(".minecraft");
    write_base(&root);
    let (forge_base, server) = forge_installer_server().await;
    let catalog = LoaderCatalog::new(LoaderCatalogConfig {
        fabric_base: forge_base.clone(),
        quilt_base: forge_base.clone(),
        forge_metadata_base: forge_base.clone(),
        forge_maven_base: forge_base.clone(),
        neoforge_api_base: forge_base.clone(),
        neoforge_maven_base: forge_base,
        user_agent: "RMCL-test".to_owned(),
        timeout: std::time::Duration::from_secs(5),
    })
    .unwrap();
    let java_path = temporary.path().join("java/bin/java");
    write_fake_java(&java_path);
    let mut java_config = JavaServiceConfig::new(temporary.path().join("java-store"));
    java_config
        .managed_roots
        .push(temporary.path().join("java"));
    java_config.include_java_home = false;
    java_config.include_path = false;
    java_config.auto_install = false;
    let java = JavaService::new(java_config).unwrap();
    let creator = InstanceCreator::new(installer(), catalog).with_forge(
        ForgeInstaller::new(DownloadConfig::default()).unwrap(),
        java,
    );

    let outcome = creator
        .create(
            request(
                &root,
                "forge-test",
                Some(LoaderSelection {
                    kind: LoaderKind::Forge,
                    version: "52.1.20".to_owned(),
                    installer_version: "1.21.1-52.1.20".to_owned(),
                }),
            ),
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap();

    let instance = outcome.install.instance;
    assert_eq!(instance.isolation(), IsolationMode::Isolated);
    assert_eq!(
        outcome.install.version.main_class.as_deref(),
        Some("example.ForgeMain")
    );
    let manifest = instance.load_manifest().unwrap().unwrap();
    assert_eq!(
        manifest.loader.as_ref().map(|loader| loader.kind),
        Some(LoaderKind::Forge)
    );
    assert_eq!(manifest.loader.as_ref().unwrap().version, "52.1.20");
    let json = serde_json::from_slice::<serde_json::Value>(
        &std::fs::read(instance.version_json_path()).unwrap(),
    )
    .unwrap();
    assert_eq!(json["id"], "forge-test");
    assert_eq!(json["inheritsFrom"], "1.21.1");
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
        ..LoaderCatalogConfig::official().unwrap()
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
        ..LoaderCatalogConfig::official().unwrap()
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
