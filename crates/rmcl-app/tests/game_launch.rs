#![cfg(unix)]

use std::{path::Path, time::Duration};

use axum::{Router, routing::get};
use rmcl_app::{
    AppError, GameLaunchEvent, GameLaunchRequest, GameLauncher, JavaSelectionOrigin, JavaService,
    JavaServiceConfig, TaskEvent, TaskId, TaskManager, TaskPhase, TaskStatus,
    VanillaInstallRequest, VanillaInstaller, VanillaSources, VersionSource,
};
use rmcl_auth::OfflineAccount;
use rmcl_domain::{OperatingSystem, Platform, ServerAddress};
use rmcl_launch::LaunchAccount;
use rmcl_net::DownloadConfig;
use serde_json::json;
use sha1::{Digest, Sha1};
use tempfile::TempDir;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use url::Url;

#[derive(Clone, Copy)]
enum ScriptMode {
    Exit,
    Failure,
    Sleep,
    Noisy,
}

struct Fixture {
    _temporary: TempDir,
    root: std::path::PathBuf,
    installer: VanillaInstaller,
    java: JavaService,
    java_path: std::path::PathBuf,
    request: GameLaunchRequest,
}

fn fixture(mode: ScriptMode) -> Fixture {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path().join(".minecraft");
    write_metadata(&root);
    let managed = temporary.path().join("managed");
    let java_path = managed.join("bin/java");
    write_java(&java_path, mode);
    let installer = VanillaInstaller::new(
        DownloadConfig::default(),
        VanillaSources {
            manifest_url: Url::parse("https://example.invalid/manifest.json").unwrap(),
            asset_object_base: Url::parse("https://example.invalid/objects/").unwrap(),
        },
    )
    .unwrap();
    let mut java_config = JavaServiceConfig::new(temporary.path().join("store"));
    java_config.managed_roots.push(managed);
    java_config.include_java_home = false;
    java_config.include_path = false;
    java_config.auto_install = false;
    let java = JavaService::new(java_config).unwrap();
    let install = VanillaInstallRequest {
        minecraft_root: root.clone(),
        version_id: "test".to_owned(),
        platform: Platform::new(OperatingSystem::Linux, "6.1", "x86_64"),
        source: VersionSource::Existing,
    };
    let request = GameLaunchRequest {
        install,
        account: LaunchAccount::from_offline(&OfflineAccount::new("Steve").unwrap()),
        java_executable: None,
        max_memory_mb: None,
        additional_jvm_arguments: Vec::new(),
        additional_game_arguments: Vec::new(),
        authlib: None,
        minecraft_version: "1.21.1".to_owned(),
        server: None,
        quick_play_world: None,
        resolution: None,
        demo: false,
    };
    Fixture {
        _temporary: temporary,
        root,
        installer,
        java,
        java_path,
        request,
    }
}

fn write_metadata(root: &Path) {
    let client = b"client";
    let index = b"{}";
    let native = native_jar();
    let version = serde_json::to_vec(&json!({
        "id": "test",
        "mainClass": "example.Main",
        "javaVersion": {"component": "java-runtime-delta", "majorVersion": 21},
        "arguments": {
            "jvm": ["-cp", "${classpath}"],
            "game": ["--username", "${auth_player_name}", "--accessToken", "${auth_access_token}"]
        },
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
        "libraries": [{
            "name": "com.example:native:1.0",
            "natives": {"linux": "natives-linux"},
            "downloads": {
                "classifiers": {
                    "natives-linux": {
                        "path": "com/example/native/1.0/native-1.0-natives-linux.jar",
                        "url": "https://example.invalid/native-linux.jar",
                        "sha1": sha1(&native),
                        "size": native.len()
                    }
                }
            },
            "extract": {"exclude": ["META-INF/"]}
        }]
    }))
    .unwrap();
    std::fs::create_dir_all(root.join("versions/test")).unwrap();
    std::fs::create_dir_all(root.join("assets/indexes")).unwrap();
    std::fs::write(root.join("versions/test/test.json"), version).unwrap();
    std::fs::write(root.join("versions/test/test.jar"), client).unwrap();
    std::fs::write(root.join("assets/indexes/test-index.json"), index).unwrap();

    let native_path = root.join("libraries/com/example/native/1.0/native-1.0-natives-linux.jar");
    std::fs::create_dir_all(native_path.parent().unwrap()).unwrap();
    std::fs::write(native_path, native).unwrap();

    let natives = root.join("versions/test/natives-linux-x86_64");
    std::fs::create_dir_all(&natives).unwrap();
    std::fs::write(natives.join("libtest.so"), b"native").unwrap();
}

fn native_jar() -> Vec<u8> {
    use std::io::Write as _;
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    let mut writer = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    writer.start_file("libtest.so", options).unwrap();
    writer.write_all(b"native").unwrap();
    writer.start_file("META-INF/ignored.txt", options).unwrap();
    writer.write_all(b"ignored").unwrap();
    writer.finish().unwrap().into_inner()
}

fn write_java(path: &Path, mode: ScriptMode) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let probe = "#!/bin/sh\nif [ \"$1\" = \"-XshowSettings:properties\" ]; then\n  printf 'java.version = 21.0.7\\nos.name = Linux\\nos.arch = amd64\\njava.vendor = TestVendor\\n' >&2\n  exit 0\nfi\n";
    let game = match mode {
        ScriptMode::Exit => {
            "printf '%s\\n' \"$@\" > raw-args.txt\nfor argument in \"$@\"; do\n  printf 'out:%s\\n' \"$argument\"\ndone\nprintf 'err:%s\\n' \"$*\" >&2\nexit 0\n"
        }
        ScriptMode::Failure => "echo failure >&2\nexit 7\n",
        ScriptMode::Sleep => "echo ready\nexec sleep 60\n",
        ScriptMode::Noisy => {
            "i=0\nwhile [ \"$i\" -lt 1100 ]; do\n  echo \"line-$i\"\n  i=$((i + 1))\ndone\nexit 0\n"
        }
    };
    std::fs::write(path, format!("{probe}{game}")).unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
}

fn write_java_8(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        "#!/bin/sh\nif [ \"$1\" = \"-XshowSettings:properties\" ]; then\n  printf 'java.version = 1.8.0_402\\nos.name = Linux\\nos.arch = amd64\\njava.vendor = Legacy\\n' >&2\n  exit 0\nfi\nexit 0\n",
    )
    .unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
}

fn drain(receiver: &mut mpsc::Receiver<GameLaunchEvent>) -> Vec<GameLaunchEvent> {
    let mut events = Vec::new();
    while let Ok(event) = receiver.try_recv() {
        events.push(event);
    }
    events
}

async fn manifest_server(version: Vec<u8>) -> (Url, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let manifest = serde_json::to_vec(&json!({
        "versions": [{
            "id": "test",
            "type": "release",
            "url": base.join("version.json").unwrap().to_string(),
            "sha1": sha1(&version)
        }]
    }))
    .unwrap();
    let router = Router::new()
        .route(
            "/manifest.json",
            get(move || {
                let manifest = manifest.clone();
                async move { manifest }
            }),
        )
        .route(
            "/version.json",
            get(move || {
                let version = version.clone();
                async move { version }
            }),
        );
    let handle = tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            eprintln!("test manifest server failed: {error}");
        }
    });
    (base.join("manifest.json").unwrap(), handle)
}

#[tokio::test]
async fn direct_launch_collects_redacted_logs_and_exit() {
    let fixture = fixture(ScriptMode::Exit);
    let launcher = GameLauncher::new(fixture.installer, fixture.java);
    let (sender, mut receiver) = mpsc::channel(512);

    let outcome = launcher
        .run(fixture.request, CancellationToken::new(), Some(sender))
        .await
        .unwrap();

    assert!(outcome.exit.success);
    assert!(!outcome.stop_requested);
    let raw = std::fs::read_to_string(fixture.root.join("raw-args.txt")).unwrap();
    let token_position = raw
        .lines()
        .position(|line| line == "--accessToken")
        .unwrap();
    let token = raw.lines().nth(token_position + 1).unwrap();
    assert_eq!(token.len(), 32);
    let events = drain(&mut receiver);
    let logs = events
        .iter()
        .filter_map(|event| match event {
            GameLaunchEvent::Output(log) => Some(log.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(logs.contains("example.Main"));
    assert!(logs.contains("<redacted>"));
    assert!(!logs.contains(token));
    assert!(events.iter().any(|event| matches!(
        event,
        GameLaunchEvent::Exited(exit) if exit.success
    )));
}

#[tokio::test]
async fn instance_memory_and_resolution_reach_the_launch_plan() {
    let mut fixture = fixture(ScriptMode::Exit);
    fixture.request.max_memory_mb = Some(4096);
    fixture.request.resolution = Some((1280, 720));
    let launcher = GameLauncher::new(fixture.installer, fixture.java);

    let outcome = launcher
        .run_installed(fixture.request, CancellationToken::new(), None)
        .await
        .unwrap();

    assert!(outcome.exit.success);
    let raw = std::fs::read_to_string(fixture.root.join("raw-args.txt")).unwrap();
    assert!(raw.lines().any(|line| line == "-Xmx4096M"));
}

#[tokio::test]
async fn quick_play_server_reaches_the_launch_plan() {
    let mut fixture = fixture(ScriptMode::Exit);
    fixture.request.minecraft_version = "1.21.1".to_owned();
    fixture.request.server = Some(ServerAddress::parse("play.example.com:25566").unwrap());
    let launcher = GameLauncher::new(fixture.installer, fixture.java);

    let outcome = launcher
        .run_installed(fixture.request, CancellationToken::new(), None)
        .await
        .unwrap();

    assert!(outcome.exit.success);
    let raw = std::fs::read_to_string(fixture.root.join("raw-args.txt")).unwrap();
    let lines = raw.lines().collect::<Vec<_>>();
    let index = lines
        .iter()
        .position(|line| *line == "--quickPlayMultiplayer")
        .unwrap();
    assert_eq!(lines[index + 1], "play.example.com:25566");
    assert!(!lines.contains(&"--server"));
}

#[tokio::test]
async fn quick_play_world_reaches_the_launch_plan() {
    let mut fixture = fixture(ScriptMode::Exit);
    fixture.request.minecraft_version = "1.21.1".to_owned();
    fixture.request.quick_play_world = Some("Survival".to_owned());
    let launcher = GameLauncher::new(fixture.installer, fixture.java);

    let outcome = launcher
        .run_installed(fixture.request, CancellationToken::new(), None)
        .await
        .unwrap();

    assert!(outcome.exit.success);
    let raw = std::fs::read_to_string(fixture.root.join("raw-args.txt")).unwrap();
    let lines = raw.lines().collect::<Vec<_>>();
    let index = lines
        .iter()
        .position(|line| *line == "--quickPlaySingleplayer")
        .unwrap();
    assert_eq!(lines[index + 1], "Survival");
}

#[tokio::test]
async fn installed_launch_skips_the_install_pipeline() {
    let fixture = fixture(ScriptMode::Exit);
    let launcher = GameLauncher::new(fixture.installer, fixture.java);
    let (sender, mut receiver) = mpsc::channel(512);

    let outcome = launcher
        .run_installed(fixture.request, CancellationToken::new(), Some(sender))
        .await
        .unwrap();

    assert!(outcome.exit.success);
    let events = drain(&mut receiver);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, GameLaunchEvent::Inspecting))
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, GameLaunchEvent::Install(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, GameLaunchEvent::Started))
    );
}

#[tokio::test]
async fn isolated_instance_launches_in_its_own_game_directory() {
    let fixture = fixture(ScriptMode::Exit);
    std::fs::write(
        fixture.root.join("versions/test/rmcl.instance.toml"),
        "schema_version = 1\nname = \"Isolated\"\nminecraft_version = \"test\"\nisolation = \"isolated\"\n",
    )
    .unwrap();
    let launcher = GameLauncher::new(fixture.installer, fixture.java);

    let outcome = launcher
        .run_installed(fixture.request, CancellationToken::new(), None)
        .await
        .unwrap();

    assert!(outcome.exit.success);
    assert!(outcome.install.instance.isolation() == rmcl_app::IsolationMode::Isolated);
    assert!(fixture.root.join("versions/test/raw-args.txt").is_file());
    assert!(!fixture.root.join("raw-args.txt").exists());
}

#[tokio::test]
async fn installed_launch_reports_missing_base_files() {
    let fixture = fixture(ScriptMode::Exit);
    std::fs::remove_file(fixture.root.join("versions/test/test.jar")).unwrap();
    let launcher = GameLauncher::new(fixture.installer, fixture.java);

    let error = launcher
        .run_installed(fixture.request, CancellationToken::new(), None)
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        AppError::InstallationIncomplete { ref version, .. } if version == "test"
    ));
}

#[tokio::test]
async fn installed_launch_materializes_missing_natives_from_existing_archives() {
    let fixture = fixture(ScriptMode::Exit);
    let natives = fixture.root.join("versions/test/natives-linux-x86_64");
    std::fs::remove_dir_all(&natives).unwrap();
    let launcher = GameLauncher::new(fixture.installer, fixture.java);

    let outcome = launcher
        .run_installed(fixture.request, CancellationToken::new(), None)
        .await
        .unwrap();

    assert!(outcome.exit.success);
    assert_eq!(
        std::fs::read(natives.join("libtest.so")).unwrap(),
        b"native"
    );
    assert!(!natives.join("META-INF").exists());
}

async fn wait_for_phase(
    receiver: &mut broadcast::Receiver<TaskEvent>,
    id: TaskId,
    phase: TaskPhase,
) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match receiver.recv().await {
                Ok(TaskEvent::Snapshot(snapshot))
                    if snapshot.id == id && snapshot.phase == Some(phase) =>
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

async fn wait_for_status(
    receiver: &mut broadcast::Receiver<TaskEvent>,
    id: TaskId,
    status: TaskStatus,
) -> rmcl_app::TaskSnapshot {
    tokio::time::timeout(Duration::from_secs(5), async {
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

#[tokio::test]
async fn manifest_game_task_stops_drains_logs_and_releases_version() {
    let mut fixture = fixture(ScriptMode::Sleep);
    let version_json = std::fs::read(fixture.root.join("versions/test/test.json")).unwrap();
    let (manifest_url, server) = manifest_server(version_json).await;
    fixture.request.install.source = VersionSource::Manifest;
    let installer = VanillaInstaller::new(
        DownloadConfig::default(),
        VanillaSources {
            manifest_url,
            asset_object_base: Url::parse("https://example.invalid/objects/").unwrap(),
        },
    )
    .unwrap();
    let manager = TaskManager::with_java(installer, fixture.java, 32);
    let mut events = manager.subscribe();
    let id = manager.start_launch(fixture.request.clone()).await.unwrap();
    wait_for_phase(&mut events, id, TaskPhase::RunningGame).await;

    let duplicate = manager.start_install(fixture.request.install.clone()).await;
    assert!(matches!(
        duplicate,
        Err(AppError::TaskAlreadyRunning { existing }) if existing == id
    ));
    assert!(manager.cancel(id).await);
    wait_for_status(&mut events, id, TaskStatus::Cancelling).await;
    let exited = wait_for_status(&mut events, id, TaskStatus::Exited).await;
    assert!(exited.stop_requested);
    assert!(exited.exit.is_some_and(|exit| !exit.success));
    assert!(exited.error.is_none());
    assert!(manager.logs(id).await.is_some());

    let install = manager
        .start_install(fixture.request.install)
        .await
        .unwrap();
    wait_for_status(&mut events, install, TaskStatus::Completed).await;
    server.abort();
}

#[tokio::test]
async fn nonzero_game_exit_is_not_a_launcher_failure() {
    let fixture = fixture(ScriptMode::Failure);
    let manager = TaskManager::with_java(fixture.installer, fixture.java, 32);
    let mut events = manager.subscribe();

    let id = manager.start_launch(fixture.request).await.unwrap();
    let exited = wait_for_status(&mut events, id, TaskStatus::Exited).await;

    assert_eq!(exited.exit.and_then(|exit| exit.code), Some(7));
    assert!(!exited.stop_requested);
    assert!(exited.error.is_none());
}

#[tokio::test]
async fn game_logs_are_bounded_and_do_not_depend_on_subscribers() {
    let fixture = fixture(ScriptMode::Noisy);
    let manager = TaskManager::with_java(fixture.installer, fixture.java, 8);
    let mut events = manager.subscribe();

    let id = manager.start_launch(fixture.request).await.unwrap();
    let exited = wait_for_status(&mut events, id, TaskStatus::Exited).await;

    assert!(exited.exit.is_some_and(|exit| exit.success));
    assert_eq!(exited.log_lines, 1100);
    let logs = manager.logs(id).await.unwrap();
    assert_eq!(logs.len(), 1024);
    assert_eq!(logs.first().unwrap().text, "line-76");
    assert_eq!(logs.last().unwrap().text, "line-1099");
}

#[tokio::test]
async fn explicit_java_override_is_probed_and_reported_as_custom() {
    let mut fixture = fixture(ScriptMode::Exit);
    fixture.request.java_executable = Some(fixture.java_path.clone());
    let launcher = GameLauncher::new(fixture.installer, fixture.java);

    let outcome = launcher
        .run_installed(fixture.request, CancellationToken::new(), None)
        .await
        .unwrap();

    assert_eq!(outcome.java.origin, JavaSelectionOrigin::Custom);
    assert_eq!(outcome.java.runtime.major_version, 21);
}

#[tokio::test]
async fn incompatible_explicit_java_is_rejected_instead_of_falling_back() {
    let temporary = TempDir::new().unwrap();
    let path = temporary.path().join("java8/bin/java");
    write_java_8(&path);
    let mut fixture = fixture(ScriptMode::Exit);
    fixture.request.java_executable = Some(path);
    let launcher = GameLauncher::new(fixture.installer, fixture.java);

    let error = launcher
        .run_installed(fixture.request, CancellationToken::new(), None)
        .await
        .unwrap_err();

    assert!(matches!(error, AppError::JavaOverrideIncompatible { .. }));
}

fn sha1(bytes: &[u8]) -> String {
    hex::encode(Sha1::digest(bytes))
}
