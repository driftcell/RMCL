#![cfg(unix)]

use std::{
    io::Write,
    path::{Path, PathBuf},
};

use axum::{Router, routing::get};
use rmcl_app::{ForgeInstallRequest, ForgeInstaller};
use rmcl_domain::{OperatingSystem, Platform, Sha1Hash};
use rmcl_instance::VersionId;
use rmcl_java::{JavaRuntime, JavaRuntimeSource};
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
    let router = Router::new()
        .route(
            "/installer.jar",
            get(move || {
                let installer = installer.clone();
                async move { installer }
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

#[test]
fn sha1_helper_is_stable() {
    assert_eq!(
        sha1_text(b"abc"),
        "a9993e364706816aba3e25717850c26c9cd0d89d"
    );
}
