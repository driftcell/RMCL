#![cfg(unix)]

use std::{
    io::Write,
    path::{Path, PathBuf},
};

use axum::{Router, routing::get};
use rmcl_app::{
    ForgeInstallRequest, ForgeInstaller, GameLaunchEvent, GameLaunchRequest, GameLauncher,
    JavaService, JavaServiceConfig, VanillaInstallRequest, VanillaInstaller, VersionSource,
};
use rmcl_auth::OfflineAccount;
use rmcl_domain::{OperatingSystem, Platform, Sha1Hash};
use rmcl_instance::VersionId;
use rmcl_java::{JavaRuntime, JavaRuntimeSource};
use rmcl_launch::LaunchAccount;
use rmcl_minecraft::GameVersion;
use rmcl_net::DownloadConfig;
use serde_json::json;
use sha1::{Digest, Sha1};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use url::Url;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

fn sha1(bytes: &[u8]) -> Sha1Hash {
    format!("{:x}", Sha1::digest(bytes)).parse().unwrap()
}

fn sha1_text(bytes: &[u8]) -> String {
    format!("{:x}", Sha1::digest(bytes))
}

fn platform() -> Platform {
    Platform::new(OperatingSystem::Linux, "6.1", "x86_64")
}

fn java(executable: PathBuf) -> JavaRuntime {
    JavaRuntime {
        executable,
        version: "21.0.7".to_owned(),
        major_version: 21,
        operating_system: OperatingSystem::Linux,
        architecture: rmcl_domain::Architecture::X86_64,
        vendor: None,
        source: JavaRuntimeSource::Managed,
    }
}

fn zip_file(path: &Path, entries: &[(&str, &[u8])]) {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, bytes) in entries {
        zip.start_file(name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

fn processor_jar(path: &Path) {
    zip_file(
        path,
        &[(
            "META-INF/MANIFEST.MF",
            b"Manifest-Version: 1.0\nMain-Class: example.Processor\n\n",
        )],
    );
}

fn fake_java(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        "#!/bin/sh\ninput=''\noutput=''\nwhile [ $# -gt 0 ]; do\n  case \"$1\" in\n    --input) input=\"$2\"; shift 2;;\n    --output) output=\"$2\"; shift 2;;\n    *) shift;;\n  esac\ndone\nmkdir -p \"$(dirname \"$output\")\"\ncp \"$input\" \"$output\"\n",
    )
    .unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
}

fn installer_jar(path: &Path, profile: serde_json::Value, input: &[u8], processor: &Path) {
    let installer = std::fs::File::create(path).unwrap();
    let mut zip = ZipWriter::new(installer);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("install_profile.json", options).unwrap();
    zip.write_all(profile.to_string().as_bytes()).unwrap();
    zip.start_file("version.json", options).unwrap();
    zip.write_all(
        json!({
            "id": "1.21.1-forge-test",
            "inheritsFrom": "1.21.1",
            "mainClass": "example.Main",
            "libraries": []
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    zip.start_file("data/input.bin", options).unwrap();
    zip.write_all(input).unwrap();
    zip.start_file("maven/example/processor/1.0/processor-1.0.jar", options)
        .unwrap();
    zip.write_all(&std::fs::read(processor).unwrap()).unwrap();
    zip.finish().unwrap();
}

async fn server(installer: Vec<u8>, mappings: Vec<u8>) -> (Url, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let installer_sha1 = sha1_text(&installer);
    let router = Router::new()
        .route(
            "/installer.jar",
            get(move || {
                let installer = installer.clone();
                async move { installer }
            }),
        )
        .route(
            "/installer.jar.sha1",
            get(move || {
                let hash = installer_sha1.clone();
                async move { hash }
            }),
        )
        .route(
            "/mappings.txt",
            get(move || {
                let mappings = mappings.clone();
                async move { mappings }
            }),
        );
    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (base, handle)
}

#[tokio::test]
async fn runs_download_mojmaps_and_processor_outputs() {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path().join(".minecraft");
    std::fs::create_dir_all(root.join("versions/1.21.1")).unwrap();
    std::fs::write(root.join("versions/1.21.1/1.21.1.jar"), b"client").unwrap();

    let input = b"input bytes".to_vec();
    let mappings = b"mappings bytes".to_vec();
    let processor = temporary.path().join("processor.jar");
    processor_jar(&processor);

    let mappings_hash = sha1(&mappings);
    let output_hash = sha1(&input);
    let profile = json!({
        "spec": 1,
        "minecraft": "1.21.1",
        "json": "/version.json",
        "libraries": [
            {"name": "example:processor:1.0", "url": "https://example.invalid/"}
        ],
        "data": {
            "MOJMAPS": {"client": "[net.minecraft:client:1.21.1:mappings@tsrg]"},
            "MOJMAPS_SHA": {"client": format!("'{}'", mappings_hash)},
            "BINPATCH": {"client": "/data/input.bin"},
            "OUTPUT": {"client": "[example:generated:1.0]"},
            "OUTPUT_SHA": {"client": format!("'{}'", output_hash)}
        },
        "processors": [
            {
                "jar": "example:processor:1.0",
                "classpath": [],
                "args": ["--task", "DOWNLOAD_MOJMAPS", "--version", "1.21.1", "--output", "{MOJMAPS}"],
                "outputs": {"{MOJMAPS}": "{MOJMAPS_SHA}"}
            },
            {
                "jar": "example:processor:1.0",
                "classpath": [],
                "args": ["--input", "{BINPATCH}", "--output", "{OUTPUT}"],
                "outputs": {"{OUTPUT}": "{OUTPUT_SHA}"}
            }
        ]
    });
    installer_jar(
        &temporary.path().join("installer.jar"),
        profile,
        &input,
        &processor,
    );
    let installer_bytes = std::fs::read(temporary.path().join("installer.jar")).unwrap();
    let (base, server) = server(installer_bytes, mappings.clone()).await;

    let java_path = temporary.path().join("bin/java");
    fake_java(&java_path);
    let base_version = GameVersion::from_json(
        &json!({
            "id": "1.21.1",
            "mainClass": "example.Main",
            "downloads": {
                "client_mappings": {
                    "url": base.join("mappings.txt").unwrap().to_string(),
                    "sha1": mappings_hash.to_string(),
                    "size": mappings.len()
                }
            }
        })
        .to_string(),
    )
    .unwrap();
    let installer = ForgeInstaller::new(DownloadConfig::default()).unwrap();

    let outcome = installer
        .install(
            ForgeInstallRequest {
                minecraft_root: root.clone(),
                base_version_id: VersionId::new("1.21.1").unwrap(),
                base_version,
                platform: platform(),
                java: java(java_path),
                installer_url: base.join("installer.jar").unwrap(),
            },
            CancellationToken::new(),
            &None,
        )
        .await
        .unwrap();

    assert_eq!(outcome.processor_count, 2);
    assert!(outcome.version_json.contains("1.21.1-forge-test"));
    assert_eq!(
        std::fs::read(
            root.join("libraries/net/minecraft/client/1.21.1/client-1.21.1-mappings.tsrg")
        )
        .unwrap(),
        mappings
    );
    assert_eq!(
        std::fs::read(root.join("libraries/example/generated/1.0/generated-1.0.jar")).unwrap(),
        input
    );
    server.abort();
}

#[tokio::test]
async fn rejects_legacy_installer_profiles() {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path().join(".minecraft");
    std::fs::create_dir_all(root.join("versions/1.21.1")).unwrap();
    std::fs::write(root.join("versions/1.21.1/1.21.1.jar"), b"client").unwrap();
    let installer_path = temporary.path().join("installer.jar");
    zip_file(
        &installer_path,
        &[(
            "install_profile.json",
            br#"{"install":{},"versionInfo":{}}"#,
        )],
    );
    let (base, server) = server(std::fs::read(&installer_path).unwrap(), Vec::new()).await;
    let installer = ForgeInstaller::new(DownloadConfig::default()).unwrap();

    let error = installer
        .install(
            ForgeInstallRequest {
                minecraft_root: root,
                base_version_id: VersionId::new("1.21.1").unwrap(),
                base_version: GameVersion::from_json(
                    r#"{"id":"1.21.1","mainClass":"example.Main"}"#,
                )
                .unwrap(),
                platform: platform(),
                java: java(PathBuf::from("/bin/false")),
                installer_url: base.join("installer.jar").unwrap(),
            },
            CancellationToken::new(),
            &None,
        )
        .await
        .unwrap_err();

    assert!(matches!(error, rmcl_app::AppError::LoaderInstaller { .. }));
    server.abort();
}

struct RealSmokeFixture {
    _temporary: TempDir,
    root: PathBuf,
    base_version: GameVersion,
    java: JavaRuntime,
    platform: Platform,
}

#[ignore = "downloads official Minecraft and installer artifacts"]
async fn real_smoke_fixture() -> RealSmokeFixture {
    let client = reqwest::Client::builder()
        .user_agent("RMCL-loader-smoke")
        .build()
        .unwrap();
    let temporary = TempDir::new().unwrap();
    let root = temporary.path().join(".minecraft");
    std::fs::create_dir_all(root.join("versions/1.21.1")).unwrap();

    let manifest: serde_json::Value = client
        .get("https://launchermeta.mojang.com/mc/game/version_manifest_v2.json")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let version_url = manifest["versions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|version| version["id"] == "1.21.1")
        .and_then(|version| version["url"].as_str())
        .unwrap();
    let version_json: serde_json::Value = client
        .get(version_url)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let client_url = version_json["downloads"]["client"]["url"].as_str().unwrap();
    let client_jar = client
        .get(client_url)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .bytes()
        .await
        .unwrap();
    std::fs::write(
        root.join("versions/1.21.1/1.21.1.json"),
        version_json.to_string(),
    )
    .unwrap();
    std::fs::write(root.join("versions/1.21.1/1.21.1.jar"), &client_jar).unwrap();
    let base_version = GameVersion::from_json(&version_json.to_string()).unwrap();

    let java_path = std::env::var_os("JAVA_HOME")
        .map(|home| PathBuf::from(home).join("bin/java"))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("/usr/bin/java"));
    let java = rmcl_java::probe(java_path, JavaRuntimeSource::System)
        .await
        .unwrap();
    let platform = Platform::new(
        if cfg!(target_os = "macos") {
            OperatingSystem::MacOs
        } else {
            OperatingSystem::Linux
        },
        "test",
        std::env::consts::ARCH,
    );
    RealSmokeFixture {
        _temporary: temporary,
        root,
        base_version,
        java,
        platform,
    }
}

async fn run_real_installer(fixture: &RealSmokeFixture, installer_url: &str) -> String {
    let installer = ForgeInstaller::new(DownloadConfig::default()).unwrap();
    let outcome = installer
        .install(
            ForgeInstallRequest {
                minecraft_root: fixture.root.clone(),
                base_version_id: VersionId::new("1.21.1").unwrap(),
                base_version: fixture.base_version.clone(),
                platform: fixture.platform.clone(),
                java: fixture.java.clone(),
                installer_url: Url::parse(installer_url).unwrap(),
            },
            CancellationToken::new(),
            &None,
        )
        .await
        .unwrap();
    assert!(outcome.processor_count >= 3);
    outcome.version_json
}

#[tokio::test]
async fn rejects_installer_hash_mismatch() {
    let installer = b"not really an installer".to_vec();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let router = Router::new()
        .route(
            "/installer.jar",
            get(move || {
                let installer = installer.clone();
                async move { installer }
            }),
        )
        .route(
            "/installer.jar.sha1",
            get(|| async { "0000000000000000000000000000000000000000" }),
        );
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let temporary = TempDir::new().unwrap();
    let root = temporary.path().join(".minecraft");
    std::fs::create_dir_all(root.join("versions/1.21.1")).unwrap();
    std::fs::write(root.join("versions/1.21.1/1.21.1.jar"), b"client").unwrap();
    let installer = ForgeInstaller::new(DownloadConfig::default()).unwrap();

    let error = installer
        .install(
            ForgeInstallRequest {
                minecraft_root: root,
                base_version_id: VersionId::new("1.21.1").unwrap(),
                base_version: GameVersion::from_json(
                    r#"{"id":"1.21.1","mainClass":"example.Main"}"#,
                )
                .unwrap(),
                platform: platform(),
                java: java(PathBuf::from("/bin/false")),
                installer_url: base.join("installer.jar").unwrap(),
            },
            CancellationToken::new(),
            &None,
        )
        .await
        .unwrap_err();

    assert!(matches!(error, rmcl_app::AppError::LoaderDownload { .. }));
    server.abort();
}

#[tokio::test]
#[ignore = "downloads official Minecraft and Forge artifacts"]
async fn real_forge_installer_runs_processor_chain() {
    let fixture = real_smoke_fixture().await;
    let version_json = run_real_installer(
        &fixture,
        "https://maven.minecraftforge.net/net/minecraftforge/forge/1.21.1-52.1.20/forge-1.21.1-52.1.20-installer.jar",
    )
    .await;

    assert!(version_json.contains("net.minecraftforge"));
    let generated = fixture
        .root
        .join("libraries/net/minecraftforge/forge/1.21.1-52.1.20/forge-1.21.1-52.1.20-client.jar");
    assert!(generated.is_file());
    assert!(zip::ZipArchive::new(std::fs::File::open(generated).unwrap()).is_ok());
}

#[tokio::test]
#[ignore = "downloads official Minecraft and NeoForge artifacts"]
async fn real_neoforge_installer_runs_processor_chain() {
    let fixture = real_smoke_fixture().await;
    let version_json = run_real_installer(
        &fixture,
        "https://maven.neoforged.net/releases/net/neoforged/neoforge/21.1.80/neoforge-21.1.80-installer.jar",
    )
    .await;

    assert!(version_json.contains("neoforge"));
    let generated = fixture
        .root
        .join("libraries/net/neoforged/neoforge/21.1.80/neoforge-21.1.80-client.jar");
    assert!(generated.is_file());
    assert!(zip::ZipArchive::new(std::fs::File::open(generated).unwrap()).is_ok());
}

async fn real_loader_launch(installer_url: &str, instance_id: &str, expected: &str) {
    let fixture = real_smoke_fixture().await;
    let version_json = run_real_installer(&fixture, installer_url).await;
    let mut profile: serde_json::Value = serde_json::from_str(&version_json).unwrap();
    profile["id"] = serde_json::Value::String(instance_id.to_owned());
    let instance_dir = fixture.root.join("versions").join(instance_id);
    std::fs::create_dir_all(&instance_dir).unwrap();
    std::fs::write(
        instance_dir.join(format!("{instance_id}.json")),
        serde_json::to_vec_pretty(&profile).unwrap(),
    )
    .unwrap();

    let installer = VanillaInstaller::official(DownloadConfig::default()).unwrap();
    let install_request = VanillaInstallRequest {
        minecraft_root: fixture.root.clone(),
        version_id: instance_id.to_owned(),
        platform: fixture.platform.clone(),
        source: VersionSource::Existing,
    };
    installer
        .install(install_request.clone(), CancellationToken::new(), None)
        .await
        .unwrap();

    let mut java_config = JavaServiceConfig::new(fixture.root.join("java-store"));
    java_config.include_java_home = false;
    java_config.include_path = false;
    java_config.auto_install = false;
    let java = JavaService::new(java_config).unwrap();
    let launcher = GameLauncher::new(installer, java);
    let cancellation = CancellationToken::new();
    let (sender, mut receiver) = tokio::sync::mpsc::channel(512);
    let request = GameLaunchRequest {
        install: install_request,
        account: LaunchAccount::from_offline(&OfflineAccount::new("LoaderSmoke").unwrap()),
        java_executable: Some(fixture.java.executable.clone()),
        max_memory_mb: None,
        additional_jvm_arguments: Vec::new(),
        additional_game_arguments: Vec::new(),
        minecraft_version: "1.21.1".to_owned(),
        server: None,
        resolution: None,
        demo: false,
    };
    let timer = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(120)).await;
        timer.cancel();
    });
    let run = launcher.run_installed(request, cancellation.clone(), Some(sender));
    let watch = async {
        let mut logs = Vec::new();
        while let Some(event) = receiver.recv().await {
            if let GameLaunchEvent::Output(log) = event {
                let lower = log.text.to_ascii_lowercase();
                if lower.contains("modlauncher running") {
                    cancellation.cancel();
                }
                logs.push(log.text);
            }
        }
        logs
    };
    let (outcome, logs) = tokio::join!(run, watch);
    let outcome = outcome.unwrap();
    assert!(outcome.stop_requested);
    let tail = logs
        .iter()
        .rev()
        .take(60)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        logs.join("\n").to_ascii_lowercase().contains(expected),
        "{expected} did not appear in logs; tail:\n{tail}"
    );
}

#[tokio::test]
#[ignore = "downloads assets and launches a real Minecraft window"]
async fn real_forge_instance_reaches_forge_logs() {
    real_loader_launch(
        "https://maven.minecraftforge.net/net/minecraftforge/forge/1.21.1-52.1.20/forge-1.21.1-52.1.20-installer.jar",
        "forge-smoke",
        "forge",
    )
    .await;
}

#[tokio::test]
#[ignore = "downloads assets and launches a real Minecraft window"]
async fn real_neoforge_instance_reaches_neoforge_logs() {
    real_loader_launch(
        "https://maven.neoforged.net/releases/net/neoforged/neoforge/21.1.80/neoforge-21.1.80-installer.jar",
        "neoforge-smoke",
        "neoforge",
    )
    .await;
}

#[test]
fn sha1_helper_is_stable() {
    assert_eq!(
        sha1_text(b"abc"),
        "a9993e364706816aba3e25717850c26c9cd0d89d"
    );
}
