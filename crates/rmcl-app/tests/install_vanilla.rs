use std::{
    collections::HashMap,
    io::Write,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{Router, routing::get};
use rmcl_app::{
    AppError, DownloadStage, InstallEvent, InstallPhase, IsolationMode, VanillaInstallRequest,
    VanillaInstaller, VanillaSources, VersionSource,
};
use rmcl_domain::{OperatingSystem, Platform};
use rmcl_net::DownloadConfig;
use serde_json::json;
use sha1::{Digest, Sha1};
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use url::Url;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

type Hits = Arc<Mutex<HashMap<String, usize>>>;

struct Fixture {
    base: Url,
    version_json: Vec<u8>,
    object_hashes: [String; 2],
    hits: Hits,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    fn url(&self, path: &str) -> Url {
        self.base.join(path).unwrap()
    }

    fn sources(&self) -> VanillaSources {
        VanillaSources {
            manifest_url: self.url("manifest.json"),
            asset_object_base: self.url("objects/"),
        }
    }

    fn hits(&self, path: &str) -> usize {
        self.hits.lock().unwrap().get(path).copied().unwrap_or(0)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn fixture(asset_delay: Duration) -> Fixture {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let base = Url::parse(&format!("http://{address}/")).unwrap();
    let hits = Hits::default();

    let client = b"client".to_vec();
    let library = b"library".to_vec();
    let native_one = native_archive(&[("native/a.so", b"one"), ("META-INF/MANIFEST.MF", b"skip")]);
    let native_two = native_archive(&[("native/b.so", b"two")]);
    let objects = [b"first asset".to_vec(), b"second asset".to_vec()];
    let object_hashes = [sha1(&objects[0]), sha1(&objects[1])];
    let asset_index = serde_json::to_vec(&json!({
        "objects": {
            "icons/first.txt": {"hash": object_hashes[0], "size": objects[0].len()},
            "icons/duplicate.txt": {"hash": object_hashes[0], "size": objects[0].len()},
            "icons/second.txt": {"hash": object_hashes[1], "size": objects[1].len()}
        }
    }))
    .unwrap();
    let version_json = serde_json::to_vec(&json!({
        "id": "test",
        "type": "release",
        "mainClass": "example.Main",
        "javaVersion": {"component": "java-runtime-delta", "majorVersion": 21},
        "arguments": {"jvm": ["-cp", "${classpath}"], "game": []},
        "downloads": {
            "client": download(base.join("client.jar").unwrap(), &client)
        },
        "assetIndex": {
            "id": "test-index",
            "url": base.join("index.json").unwrap().to_string(),
            "sha1": sha1(&asset_index),
            "size": asset_index.len()
        },
        "libraries": [
            {
                "name": "com.example:core:1.0",
                "downloads": {"artifact": with_path(
                    base.join("library.jar").unwrap(),
                    "com/example/core/1.0/core-1.0.jar",
                    &library
                )}
            },
            native_library(&base, "one", &native_one),
            native_library(&base, "two", &native_two)
        ]
    }))
    .unwrap();
    let manifest = serde_json::to_vec(&json!({
        "latest": {"release": "test"},
        "versions": [{
            "id": "test",
            "type": "release",
            "url": base.join("version.json").unwrap().to_string(),
            "sha1": sha1(&version_json)
        }]
    }))
    .unwrap();

    let mut router = Router::new();
    for (path, bytes, delayed) in [
        ("/manifest.json", manifest, false),
        ("/version.json", version_json.clone(), false),
        ("/client.jar", client, false),
        ("/library.jar", library, false),
        ("/native-one.jar", native_one, false),
        ("/native-two.jar", native_two, false),
        ("/index.json", asset_index, false),
        (
            Box::leak(object_path(&object_hashes[0]).into_boxed_str()),
            objects[0].clone(),
            true,
        ),
        (
            Box::leak(object_path(&object_hashes[1]).into_boxed_str()),
            objects[1].clone(),
            true,
        ),
    ] {
        router = route_bytes(
            router,
            path,
            bytes,
            hits.clone(),
            delayed.then_some(asset_delay),
        );
    }
    let server = tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            eprintln!("test server failed: {error}");
        }
    });
    Fixture {
        base,
        version_json,
        object_hashes,
        hits,
        server,
    }
}

fn download(url: Url, bytes: &[u8]) -> serde_json::Value {
    json!({"url": url.to_string(), "sha1": sha1(bytes), "size": bytes.len()})
}

fn with_path(url: Url, path: &str, bytes: &[u8]) -> serde_json::Value {
    let mut value = download(url, bytes);
    value["path"] = json!(path);
    value
}

fn native_library(base: &Url, name: &str, bytes: &[u8]) -> serde_json::Value {
    json!({
        "name": format!("com.example:native-{name}:1.0"),
        "natives": {"linux": "natives-linux"},
        "extract": {"exclude": ["META-INF/"]},
        "downloads": {
            "classifiers": {
                "natives-linux": with_path(
                    base.join(&format!("native-{name}.jar")).unwrap(),
                    &format!("com/example/native-{name}/1.0/native-{name}-1.0-natives-linux.jar"),
                    bytes
                )
            }
        }
    })
}

fn native_archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        writer
            .start_file(
                *name,
                SimpleFileOptions::default()
                    .compression_method(CompressionMethod::Stored)
                    .unix_permissions(0o100644),
            )
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn route_bytes(
    router: Router,
    path: &'static str,
    bytes: Vec<u8>,
    hits: Hits,
    delay: Option<Duration>,
) -> Router {
    let observed_path = path.to_owned();
    router.route(
        path,
        get(move || {
            let hits = hits.clone();
            let bytes = bytes.clone();
            let observed_path = observed_path.clone();
            async move {
                *hits.lock().unwrap().entry(observed_path).or_default() += 1;
                if let Some(delay) = delay {
                    tokio::time::sleep(delay).await;
                }
                bytes
            }
        }),
    )
}

fn object_path(hash: &str) -> String {
    format!("/objects/{}/{hash}", &hash[..2])
}

fn sha1(bytes: &[u8]) -> String {
    hex::encode(Sha1::digest(bytes))
}

fn config() -> DownloadConfig {
    DownloadConfig {
        max_concurrency: 2,
        max_concurrency_per_host: 2,
        max_attempts_per_source: 1,
        connect_timeout: Duration::from_secs(1),
        request_timeout: Duration::from_secs(2),
        initial_retry_delay: Duration::from_millis(1),
        max_retry_delay: Duration::from_millis(5),
    }
}

fn request(root: &Path, source: VersionSource) -> VanillaInstallRequest {
    VanillaInstallRequest {
        minecraft_root: root.to_path_buf(),
        version_id: "test".to_owned(),
        platform: Platform::new(OperatingSystem::Linux, "6.1", "x86_64"),
        source,
    }
}

fn drain_events(receiver: &mut mpsc::Receiver<InstallEvent>) -> Vec<InstallEvent> {
    let mut events = Vec::new();
    while let Ok(event) = receiver.try_recv() {
        events.push(event);
    }
    events
}

#[tokio::test]
async fn installs_remote_vanilla_and_reuses_verified_files() {
    let fixture = fixture(Duration::ZERO).await;
    let temporary = TempDir::new().unwrap();
    let root = temporary.path().join(".minecraft");
    let installer = VanillaInstaller::new(config(), fixture.sources()).unwrap();
    let (sender, mut receiver) = mpsc::channel(512);

    let outcome = installer
        .install(
            request(&root, VersionSource::Manifest),
            CancellationToken::new(),
            Some(sender),
        )
        .await
        .unwrap();

    assert_eq!(outcome.version.id, "test");
    assert_eq!(outcome.instance.isolation(), IsolationMode::Isolated);
    assert_eq!(outcome.instance.game_dir(), root.join("versions/test"));
    assert!(
        outcome
            .instance
            .load_manifest()
            .unwrap()
            .is_some_and(|manifest| manifest.isolation == IsolationMode::Isolated)
    );
    assert!(outcome.instance.mods_dir().is_dir());
    assert_eq!(outcome.base_plan.files.len(), 5);
    assert_eq!(outcome.asset_plan.files.len(), 2);
    assert_eq!(outcome.downloads.downloaded_files, 7);
    assert_eq!(outcome.downloads.skipped_files, 0);
    assert_eq!(outcome.natives.len(), 2);
    assert_eq!(
        std::fs::read(root.join("versions/test/test.json")).unwrap(),
        fixture.version_json
    );
    assert_eq!(
        std::fs::read(root.join("versions/test/test.jar")).unwrap(),
        b"client"
    );
    assert_eq!(
        std::fs::read(root.join("libraries/com/example/core/1.0/core-1.0.jar")).unwrap(),
        b"library"
    );
    let natives = root.join("versions/test/natives-linux-x86_64");
    assert_eq!(std::fs::read(natives.join("native/a.so")).unwrap(), b"one");
    assert_eq!(std::fs::read(natives.join("native/b.so")).unwrap(), b"two");
    assert!(!natives.join("META-INF").exists());
    for (hash, expected) in fixture
        .object_hashes
        .iter()
        .zip([b"first asset".as_slice(), b"second asset".as_slice()])
    {
        assert_eq!(
            std::fs::read(root.join(format!("assets/objects/{}/{hash}", &hash[..2]))).unwrap(),
            expected
        );
    }

    let events = drain_events(&mut receiver);
    let phases = events
        .iter()
        .filter_map(|event| match event {
            InstallEvent::Phase(phase) => Some(*phase),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        [
            InstallPhase::ResolvingVersion,
            InstallPhase::PersistingVersion,
            InstallPhase::PlanningBase,
            InstallPhase::DownloadingBase,
            InstallPhase::ReadingAssetIndex,
            InstallPhase::PlanningAssets,
            InstallPhase::DownloadingAssets,
            InstallPhase::MergingNatives,
            InstallPhase::Completed,
        ]
    );
    assert!(events.iter().any(|event| matches!(
        event,
        InstallEvent::Natives {
            completed_archives: 2,
            total_archives: 2,
            ..
        }
    )));

    let second = installer
        .install(
            request(&root, VersionSource::Manifest),
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(second.downloads.downloaded_files, 0);
    assert_eq!(second.downloads.skipped_files, 7);
    assert_eq!(fixture.hits("/manifest.json"), 2);
    let object_routes = [
        object_path(&fixture.object_hashes[0]),
        object_path(&fixture.object_hashes[1]),
    ];
    for path in [
        "/version.json",
        "/client.jar",
        "/library.jar",
        "/native-one.jar",
        "/native-two.jar",
        "/index.json",
        object_routes[0].as_str(),
        object_routes[1].as_str(),
    ] {
        assert_eq!(fixture.hits(path), 1, "unexpected requests for {path}");
    }
}

#[tokio::test]
async fn cancellation_during_assets_is_not_a_failure() {
    let fixture = fixture(Duration::from_secs(5)).await;
    let temporary = TempDir::new().unwrap();
    let root = temporary.path().join(".minecraft");
    let installer = VanillaInstaller::new(config(), fixture.sources()).unwrap();
    let cancellation = CancellationToken::new();
    let task_cancellation = cancellation.clone();
    let task_installer = installer.clone();
    let task_root = root.clone();
    let (sender, mut receiver) = mpsc::channel(512);
    let task = tokio::spawn(async move {
        task_installer
            .install(
                request(&task_root, VersionSource::Manifest),
                task_cancellation,
                Some(sender),
            )
            .await
    });

    while let Some(event) = receiver.recv().await {
        if matches!(
            event,
            InstallEvent::Download(ref progress)
                if progress.phase == InstallPhase::DownloadingAssets
                    && progress.stage == DownloadStage::Downloading
        ) {
            cancellation.cancel();
            break;
        }
    }
    let result = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();

    assert!(matches!(result, Err(AppError::Cancelled)));
    assert!(root.join("versions/test/test.json").is_file());
    assert!(
        !root
            .join("versions/test/natives-linux-x86_64/native/a.so")
            .exists()
    );
}

#[tokio::test]
async fn local_version_json_is_validated_before_publishing_original_bytes() {
    let fixture = fixture(Duration::ZERO).await;
    let temporary = TempDir::new().unwrap();
    let root = temporary.path().join(".minecraft");
    let source = temporary.path().join("source.json");
    let mut original = fixture.version_json.clone();
    original.extend_from_slice(b"\n  \n");
    std::fs::write(&source, &original).unwrap();
    let installer = VanillaInstaller::new(config(), fixture.sources()).unwrap();

    installer
        .install(
            request(&root, VersionSource::LocalJson(source.clone())),
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap();

    let destination = root.join("versions/test/test.json");
    assert_eq!(std::fs::read(&destination).unwrap(), original);
    std::fs::write(&source, b"not JSON").unwrap();
    let result = installer
        .install(
            request(&root, VersionSource::LocalJson(source)),
            CancellationToken::new(),
            None,
        )
        .await;
    assert!(matches!(result, Err(AppError::Metadata(_))));
    assert_eq!(std::fs::read(destination).unwrap(), original);
}
