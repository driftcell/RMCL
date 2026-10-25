use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use axum::{Json, Router, response::IntoResponse, routing::get};
use rmcl_domain::{Architecture, OperatingSystem};
use rmcl_java::{
    JavaError, JavaRequirement, MarkerFile, MojangDownload, MojangFile, MojangFileDownloads,
    MojangFileType, MojangRuntimeEntry, MojangRuntimeIndex, MojangRuntimeInstaller,
    MojangRuntimeManifest, MojangRuntimeVersion, RuntimeInstallLock, RuntimeMarker, UrlPolicy,
};
use sha1::{Digest, Sha1};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use url::Url;

const HASH_A: &str = "0123456789abcdef0123456789abcdef01234567";
const HASH_B: &str = "abcdefabcdefabcdefabcdefabcdefabcdefabcd";

#[derive(Clone)]
struct MockData {
    index: MojangRuntimeIndex,
    manifest: MojangRuntimeManifest,
    java: Vec<u8>,
    example: Vec<u8>,
}

impl MockData {
    fn with_base(self, base: &Url) -> Self {
        let mut this = self;
        for (name, file) in this.manifest.files.iter_mut() {
            if let Some(downloads) = file.downloads.as_mut() {
                if let Some(raw) = downloads.raw.as_mut() {
                    raw.url = base.join(name).unwrap().to_string();
                }
                if let Some(lzma) = downloads.lzma.as_mut() {
                    lzma.url = base.join(&format!("{name}.lzma")).unwrap().to_string();
                }
            }
        }
        let manifest_bytes = serde_json::to_vec(&this.manifest).unwrap();
        for entries in this.index.values_mut() {
            for entry in entries.values_mut().flatten() {
                entry.manifest.url = base.join("manifest.json").unwrap().to_string();
                if entry.manifest.sha1.is_none() {
                    entry.manifest.sha1 = Some(sha1_hex(&manifest_bytes));
                }
                if entry.manifest.size.is_none() {
                    entry.manifest.size = Some(manifest_bytes.len() as u64);
                }
            }
        }
        this
    }
}

struct MockServer {
    base: Url,
    hits: Arc<AtomicUsize>,
    handle: tokio::task::JoinHandle<()>,
}

impl MockServer {
    fn url(&self, path: &str) -> Url {
        self.base.join(path).unwrap()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn serve(data: MockData) -> MockServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let data = data.with_base(&base);
    let hits = Arc::new(AtomicUsize::new(0));
    let router = Router::new()
        .route(
            "/index.json",
            get({
                let data = data.clone();
                let hits = hits.clone();
                move || {
                    let data = data.clone();
                    let hits = hits.clone();
                    async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        Json(data.index).into_response()
                    }
                }
            }),
        )
        .route(
            "/manifest.json",
            get({
                let data = data.clone();
                let hits = hits.clone();
                move || {
                    let data = data.clone();
                    let hits = hits.clone();
                    async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        Json(data.manifest).into_response()
                    }
                }
            }),
        )
        .route(
            "/bin/java",
            get({
                let data = data.clone();
                let hits = hits.clone();
                move || {
                    let data = data.clone();
                    let hits = hits.clone();
                    async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        data.java.into_response()
                    }
                }
            }),
        )
        .route(
            "/jre.bundle/Contents/Home/bin/java",
            get({
                let data = data.clone();
                let hits = hits.clone();
                move || {
                    let data = data.clone();
                    let hits = hits.clone();
                    async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        data.java.into_response()
                    }
                }
            }),
        )
        .route(
            "/lib/example.txt",
            get({
                let data = data.clone();
                let hits = hits.clone();
                move || {
                    let data = data.clone();
                    let hits = hits.clone();
                    async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        data.example.into_response()
                    }
                }
            }),
        );
    let handle = tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            eprintln!("mock runtime server failed: {error}");
        }
    });
    MockServer { base, hits, handle }
}

fn mock_data() -> MockData {
    mock_data_with("linux", "java-runtime-delta", "21.0.7", "Linux", "amd64")
}

fn mock_data_with(
    platform: &str,
    component: &str,
    version: &str,
    os_name: &str,
    arch: &str,
) -> MockData {
    let java = format!(
        "#!/bin/sh\nprintf 'java.version = {version}\nos.name = {os_name}\nos.arch = {arch}\njava.vendor = TestVendor\n' >&2\n"
    )
    .into_bytes();
    let example = b"hello".to_vec();
    let manifest = MojangRuntimeManifest {
        files: BTreeMap::from([
            (
                "bin/java".to_owned(),
                MojangFile {
                    kind: MojangFileType::File,
                    downloads: Some(MojangFileDownloads {
                        raw: Some(download(&java)),
                        lzma: None,
                    }),
                    executable: Some(true),
                    target: None,
                },
            ),
            (
                "lib/example.txt".to_owned(),
                MojangFile {
                    kind: MojangFileType::File,
                    downloads: Some(MojangFileDownloads {
                        raw: Some(download(&example)),
                        lzma: None,
                    }),
                    executable: Some(false),
                    target: None,
                },
            ),
            (
                "legal".to_owned(),
                MojangFile {
                    kind: MojangFileType::Directory,
                    downloads: None,
                    executable: None,
                    target: None,
                },
            ),
            (
                "legal/readme".to_owned(),
                MojangFile {
                    kind: MojangFileType::Link,
                    downloads: None,
                    executable: None,
                    target: Some("../lib/example.txt".to_owned()),
                },
            ),
        ]),
    };
    MockData {
        index: BTreeMap::from([(
            platform.to_owned(),
            BTreeMap::from([(
                component.to_owned(),
                vec![MojangRuntimeEntry {
                    version: MojangRuntimeVersion {
                        name: version.to_owned(),
                        released: "2026-01-01T00:00:00+00:00".to_owned(),
                    },
                    manifest: MojangDownload {
                        url: "manifest.json".to_owned(),
                        sha1: None,
                        size: None,
                    },
                }],
            )]),
        )]),
        manifest,
        java,
        example,
    }
}

fn download(data: &[u8]) -> MojangDownload {
    MojangDownload {
        url: String::new(),
        sha1: Some(sha1_hex(data)),
        size: Some(data.len() as u64),
    }
}

fn sha1_hex(data: &[u8]) -> String {
    let digest = Sha1::digest(data);
    let mut bytes = [0_u8; 20];
    bytes.copy_from_slice(&digest);
    rmcl_domain::Sha1Hash::from_bytes(bytes).to_string()
}

fn requirement() -> JavaRequirement {
    JavaRequirement {
        component: None,
        major_version: Some(21),
        maximum_major: None,
        operating_system: OperatingSystem::Linux,
        architecture: Architecture::X86_64,
    }
}

fn installer(server: &MockServer, store: &TempDir) -> MojangRuntimeInstaller {
    MojangRuntimeInstaller::new(store.path().join("store"))
        .unwrap()
        .with_index_url(server.url("/index.json"))
        .with_url_policy(UrlPolicy::AnyHttp)
}

#[cfg(unix)]
#[tokio::test]
async fn installs_and_reuses_a_verified_runtime() {
    let server = serve(mock_data()).await;
    let store = TempDir::new().unwrap();
    let installer = installer(&server, &store);

    let runtime = installer
        .install(&requirement(), CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(runtime.major_version, 21);
    assert_eq!(runtime.operating_system, OperatingSystem::Linux);
    assert!(runtime.executable.ends_with("bin/java"));
    assert!(
        store
            .path()
            .join("store/linux/java-runtime-delta.json")
            .is_file()
    );
    let link = store
        .path()
        .join("store/linux/java-runtime-delta/legal/readme");
    assert!(link.is_symlink());
    assert_eq!(
        std::fs::read_link(link).unwrap().to_string_lossy(),
        "../lib/example.txt"
    );
    let hits = server.hits.load(Ordering::SeqCst);
    assert_eq!(hits, 4);

    let reused = installer
        .install(&requirement(), CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(reused.executable, runtime.executable);
    assert_eq!(server.hits.load(Ordering::SeqCst), hits);
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_modified_installed_files_instead_of_reusing_them() {
    let server = serve(mock_data()).await;
    let store = TempDir::new().unwrap();
    let installer = installer(&server, &store);
    let runtime = installer
        .install(&requirement(), CancellationToken::new())
        .await
        .unwrap();
    let modified = runtime
        .executable
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("lib/example.txt");
    std::fs::write(modified, b"corrupted").unwrap();

    let error = installer
        .install(&requirement(), CancellationToken::new())
        .await
        .unwrap_err();

    assert!(matches!(error, JavaError::ExistingRuntimeInvalid { .. }));
}

#[tokio::test]
async fn rejects_missing_raw_download() {
    let mut data = mock_data();
    data.manifest
        .files
        .get_mut("lib/example.txt")
        .unwrap()
        .downloads = Some(MojangFileDownloads {
        raw: None,
        lzma: Some(download(b"compressed")),
    });
    let server = serve(data).await;
    let store = TempDir::new().unwrap();

    let error = installer(&server, &store)
        .install(&requirement(), CancellationToken::new())
        .await
        .unwrap_err();

    assert!(matches!(error, JavaError::MissingRawDownload(_)));
}

#[tokio::test]
async fn rejects_unsafe_manifest_paths() {
    let mut data = mock_data();
    data.manifest.files.insert(
        "../escape".to_owned(),
        MojangFile {
            kind: MojangFileType::File,
            downloads: Some(MojangFileDownloads {
                raw: Some(download(b"bad")),
                lzma: None,
            }),
            executable: Some(false),
            target: None,
        },
    );
    let server = serve(data).await;
    let store = TempDir::new().unwrap();

    let error = installer(&server, &store)
        .install(&requirement(), CancellationToken::new())
        .await
        .unwrap_err();

    assert!(matches!(error, JavaError::Path(_)));
}

#[tokio::test]
async fn rejects_link_targets_outside_the_runtime() {
    let mut data = mock_data();
    data.manifest.files.get_mut("legal/readme").unwrap().target = Some("../../../etc".to_owned());
    let server = serve(data).await;
    let store = TempDir::new().unwrap();

    let error = installer(&server, &store)
        .install(&requirement(), CancellationToken::new())
        .await
        .unwrap_err();

    assert!(matches!(error, JavaError::InvalidLink { .. }));
}

#[tokio::test]
async fn rejects_raw_hash_mismatch() {
    let mut data = mock_data();
    data.example = b"evil".to_vec();
    let server = serve(data).await;
    let store = TempDir::new().unwrap();

    let error = installer(&server, &store)
        .install(&requirement(), CancellationToken::new())
        .await
        .unwrap_err();

    assert!(matches!(error, JavaError::DownloadFailures { .. }));
    assert!(!store.path().join("store/linux/java-runtime-delta").exists());
}

#[tokio::test]
async fn rejects_manifest_hash_mismatch() {
    let mut data = mock_data();
    for entries in data.index.values_mut() {
        for entry in entries.values_mut().flatten() {
            entry.manifest.sha1 = Some(HASH_B.to_owned());
        }
    }
    let server = serve(data).await;
    let store = TempDir::new().unwrap();

    let error = installer(&server, &store)
        .install(&requirement(), CancellationToken::new())
        .await
        .unwrap_err();

    assert!(matches!(error, JavaError::ManifestHashMismatch { .. }));
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_a_runtime_that_fails_probe() {
    let mut data = mock_data();
    data.java = b"#!/bin/sh\nexit 1\n".to_vec();
    data.manifest.files.get_mut("bin/java").unwrap().downloads = Some(MojangFileDownloads {
        raw: Some(download(&data.java)),
        lzma: None,
    });
    let server = serve(data).await;
    let store = TempDir::new().unwrap();

    let error = installer(&server, &store)
        .install(&requirement(), CancellationToken::new())
        .await
        .unwrap_err();

    assert!(matches!(error, JavaError::ProbeRejected { .. }));
    assert!(!store.path().join("store/linux/java-runtime-delta").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn installs_macos_bundle_layout() {
    let mut data = mock_data_with(
        "mac-os-arm64",
        "java-runtime-delta",
        "21.0.7",
        "Mac OS X",
        "aarch64",
    );
    let java = data.manifest.files.remove("bin/java").unwrap();
    data.manifest
        .files
        .insert("jre.bundle/Contents/Home/bin/java".to_owned(), java);
    let server = serve(data).await;
    let store = TempDir::new().unwrap();
    let installer = installer(&server, &store);
    let requirement = JavaRequirement {
        component: None,
        major_version: Some(21),
        maximum_major: None,
        operating_system: OperatingSystem::MacOs,
        architecture: Architecture::Aarch64,
    };

    let runtime = installer
        .install(&requirement, CancellationToken::new())
        .await
        .unwrap();

    assert!(
        runtime
            .executable
            .ends_with("jre.bundle/Contents/Home/bin/java")
    );
    assert_eq!(runtime.operating_system, OperatingSystem::MacOs);
    assert!(
        store
            .path()
            .join("store/mac-os-arm64/java-runtime-delta.json")
            .is_file()
    );
}

#[test]
fn install_lock_is_exclusive_and_released() {
    let store = TempDir::new().unwrap();
    let first = RuntimeInstallLock::acquire(store.path(), "linux", "java-runtime-delta").unwrap();
    let error =
        RuntimeInstallLock::acquire(store.path(), "linux", "java-runtime-delta").unwrap_err();

    assert!(matches!(error, JavaError::AlreadyInstalling(_)));
    drop(first);
    let second = RuntimeInstallLock::acquire(store.path(), "linux", "java-runtime-delta").unwrap();
    assert!(second.path().is_file());
}

#[test]
fn marker_round_trip_preserves_schema_and_files() {
    let marker = RuntimeMarker {
        schema_version: 1,
        source: "mojang".to_owned(),
        platform: "linux".to_owned(),
        component: "java-runtime-delta".to_owned(),
        requested_major: 21,
        manifest_url: "https://example.invalid/manifest.json".to_owned(),
        manifest_sha1: Some(HASH_A.to_owned()),
        manifest_size: Some(10),
        executable: "bin/java".to_owned(),
        runtime_version: "21.0.7".to_owned(),
        vendor: Some("TestVendor".to_owned()),
        installed_at: 1,
        files: BTreeMap::from([(
            "bin/java".to_owned(),
            MarkerFile::File {
                sha1: Some(HASH_A.to_owned()),
                size: Some(10),
                executable: true,
            },
        )]),
    };
    let bytes = serde_json::to_vec(&marker).unwrap();
    let decoded: RuntimeMarker = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(decoded.schema_version, 1);
    assert_eq!(decoded.component, "java-runtime-delta");
    assert_eq!(decoded.requested_major, 21);
    assert!(matches!(decoded.files["bin/java"], MarkerFile::File { .. }));
}
