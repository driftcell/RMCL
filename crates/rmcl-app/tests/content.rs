use std::{io::Write, path::Path, time::Duration};

use axum::{Json, Router, response::IntoResponse, routing::get};
use rmcl_app::{
    AppError, ConflictPolicy, ContentGraphInstallRequest, ContentInstallPhase,
    ContentInstallRequest, ContentInstaller, ContentKind, ContentSource, IsolationMode,
    ModMetadataCache, ModrinthClient, ModrinthConfig, PackFormat, RemoteDependency,
    RemoteDependencyKind, RemoteFile, RemoteVersion, ResourcePackOrder, ShaderLoader, TaskKind,
    TaskManager, TaskPhase, TaskProgress, TaskStatus, TaskSubject, VanillaInstaller,
    delete_instance_content, import_instance_content, move_instance_resource_pack,
    preflight_content_install, read_instance_shader_pack, resolve_content_install_batch,
    resource_pack_format, scan_installed_mods, scan_instance_content, set_instance_content_enabled,
    set_instance_shader_pack,
};
use rmcl_domain::Sha1Hash;
use rmcl_instance::{InstanceLayout, MinecraftLayout, RepositoryPath, VersionId};
use rmcl_net::DownloadConfig;
use serde_json::json;
use sha1::{Digest, Sha1};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use url::Url;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

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

fn write_fabric_mod(path: &Path, id: &str, version: &str) {
    write_fabric_mod_with_depends(path, id, version, json!({}));
}

fn write_fabric_mod_with_depends(path: &Path, id: &str, version: &str, depends: serde_json::Value) {
    write_fabric_mod_with_metadata(path, id, version, depends, json!({}));
}

fn write_fabric_mod_with_metadata(
    path: &Path,
    id: &str,
    version: &str,
    depends: serde_json::Value,
    breaks: serde_json::Value,
) {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("fabric.mod.json", options).unwrap();
    zip.write_all(
        json!({"id": id, "version": version, "name": id, "depends": depends, "breaks": breaks})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    zip.finish().unwrap();
}

fn fabric_mod_bytes(id: &str, version: &str) -> Vec<u8> {
    fabric_mod_bytes_with_depends(id, version, json!({}))
}

fn fabric_mod_bytes_with_depends(id: &str, version: &str, depends: serde_json::Value) -> Vec<u8> {
    let temporary = TempDir::new().unwrap();
    let path = temporary.path().join("mod.jar");
    write_fabric_mod_with_depends(&path, id, version, depends);
    std::fs::read(path).unwrap()
}

fn fabric_mod_bytes_with_breaks(id: &str, version: &str, breaks: serde_json::Value) -> Vec<u8> {
    let temporary = TempDir::new().unwrap();
    let path = temporary.path().join("mod.jar");
    write_fabric_mod_with_metadata(&path, id, version, json!({}), breaks);
    std::fs::read(path).unwrap()
}

fn write_zip_entry(path: &Path, name: &str, contents: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = std::fs::File::create(path).unwrap();
    let mut archive = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    archive.start_file(name, options).unwrap();
    archive.write_all(contents).unwrap();
    archive.finish().unwrap();
}

#[test]
fn resource_pack_format_uses_the_loader_base_client_jar() {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path().join(".minecraft");
    let base = instance(&root);
    write(
        &base.version_json_path(),
        br#"{"id":"1.21.1","jar":"1.21.1"}"#,
    );
    write_zip_entry(
        &base.client_jar_path_for(None),
        "version.json",
        br#"{"id":"1.21.1","pack_version":{"resource_major":34,"resource_minor":0}}"#,
    );
    assert_eq!(resource_pack_format(&base), Some(PackFormat::new(34, 0)));

    let loader = MinecraftLayout::new(&root).instance(
        VersionId::new("fabric-1.21.1").unwrap(),
        IsolationMode::Isolated,
    );
    write(
        &loader.version_json_path(),
        br#"{"id":"fabric-1.21.1","inheritsFrom":"1.21.1","jar":"1.21.1"}"#,
    );
    assert_eq!(resource_pack_format(&loader), Some(PackFormat::new(34, 0)));
}

#[test]
fn resource_pack_format_is_unknown_without_client_metadata() {
    let temporary = TempDir::new().unwrap();
    let instance = instance(&temporary.path().join(".minecraft"));
    assert_eq!(resource_pack_format(&instance), None);
}

#[tokio::test]
async fn applies_resource_pack_priority_through_the_app_wrapper() {
    let temporary = TempDir::new().unwrap();
    let instance = instance(&temporary.path().join(".minecraft"));
    write(&instance.resourcepacks_dir().join("a.zip"), b"a");
    write(&instance.resourcepacks_dir().join("b.zip"), b"b");
    write(
        &instance.options_path(),
        b"resourcePacks:[\"file/a.zip\",\"file/b.zip\"]\n",
    );

    move_instance_resource_pack(
        instance.clone(),
        RepositoryPath::new("a.zip").unwrap(),
        ResourcePackOrder::HigherPriority,
    )
    .await
    .unwrap();
    assert!(
        std::fs::read_to_string(instance.options_path())
            .unwrap()
            .contains("resourcePacks:[\"file/b.zip\",\"file/a.zip\"]")
    );
}

#[tokio::test]
async fn selects_and_disables_a_shader_pack_through_the_app_wrapper() {
    let temporary = TempDir::new().unwrap();
    let instance = instance(&temporary.path().join(".minecraft"));
    write(&instance.shaderpacks_dir().join("Nice.zip"), b"shader");

    assert_eq!(
        read_instance_shader_pack(instance.clone(), ShaderLoader::Iris)
            .await
            .unwrap(),
        None
    );
    set_instance_shader_pack(
        instance.clone(),
        ShaderLoader::Iris,
        Some("Nice.zip".to_owned()),
    )
    .await
    .unwrap();
    let selection = read_instance_shader_pack(instance.clone(), ShaderLoader::Iris)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(selection.active.as_deref(), Some("Nice.zip"));
    assert!(selection.enabled);

    set_instance_shader_pack(instance.clone(), ShaderLoader::Iris, None)
        .await
        .unwrap();
    let selection = read_instance_shader_pack(instance, ShaderLoader::Iris)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(selection.active.as_deref(), Some("Nice.zip"));
    assert!(!selection.enabled);
}

fn sha1(bytes: &[u8]) -> Sha1Hash {
    hex::encode(Sha1::digest(bytes)).parse().unwrap()
}

fn sha1_text(bytes: &[u8]) -> String {
    hex::encode(Sha1::digest(bytes))
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
                replace: None,
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
                replace: None,
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
async fn installs_remote_resource_and_shader_packs() {
    let bytes = b"remote pack".to_vec();
    let (base, server) = mod_server(bytes.clone()).await;
    let temporary = TempDir::new().unwrap();
    let instance = instance(&temporary.path().join(".minecraft"));
    let installer = ContentInstaller::official(DownloadConfig::default()).unwrap();

    for (kind, filename) in [
        (ContentKind::ResourcePack, "pack.zip"),
        (ContentKind::ShaderPack, "shader.zip"),
    ] {
        let outcome = installer
            .install(
                ContentInstallRequest {
                    instance: instance.clone(),
                    kind,
                    file: RemoteFile {
                        filename: filename.to_owned(),
                        url: base.join("mod.jar").unwrap(),
                        size: Some(bytes.len() as u64),
                        sha1: Some(sha1(&bytes)),
                        primary: true,
                    },
                    display_name: filename.to_owned(),
                    conflict: ConflictPolicy::Rename,
                    replace: None,
                },
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();

        assert_eq!(outcome.content.relative_path.as_str(), filename);
        assert_eq!(outcome.content.enabled, kind == ContentKind::ShaderPack);
    }

    set_instance_content_enabled(
        instance.clone(),
        ContentKind::ResourcePack,
        RepositoryPath::new("pack.zip").unwrap(),
        true,
    )
    .await
    .unwrap();
    assert!(
        std::fs::read_to_string(instance.options_path())
            .unwrap()
            .contains("resourcePacks:[\"file/pack.zip\"]")
    );
    server.abort();
}

#[tokio::test]
async fn replacement_install_preserves_relative_path_and_enabled_state() {
    let bytes = b"new remote mod".to_vec();
    let (base, server) = mod_server(bytes.clone()).await;
    let temporary = TempDir::new().unwrap();
    let instance = instance(&temporary.path().join(".minecraft"));
    let destination = instance.mods_dir().join("example.jar.disabled");
    write(&destination, b"old remote mod");
    let installer = ContentInstaller::official(DownloadConfig::default()).unwrap();

    let outcome = installer
        .install(
            ContentInstallRequest {
                instance: instance.clone(),
                kind: ContentKind::Mod,
                file: remote_file(&base, &bytes, None),
                display_name: "Example".to_owned(),
                conflict: ConflictPolicy::Rename,
                replace: Some(RepositoryPath::new("example.jar.disabled").unwrap()),
            },
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap();

    assert_eq!(
        outcome.content.relative_path.as_str(),
        "example.jar.disabled"
    );
    assert!(!outcome.content.enabled);
    assert_eq!(std::fs::read(destination).unwrap(), bytes);
    assert_eq!(
        scan_instance_content(instance)
            .await
            .unwrap()
            .count(ContentKind::Mod),
        1
    );
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
                replace: None,
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
                replace: None,
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
                replace: None,
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

fn project_json(id: &str, slug: &str, title: &str) -> serde_json::Value {
    json!({
        "id": id,
        "slug": slug,
        "title": title,
        "description": title,
        "categories": ["fabric"],
        "project_type": "mod",
        "downloads": 0,
        "follows": 0,
        "game_versions": ["1.21.1"],
        "loaders": ["fabric"],
        "updated": "2026-01-01T00:00:00Z"
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
    let primary_project = project_json("primary-project", "primary-mod", "Primary Mod");
    let dependency_project = project_json("dep-project", "dependency-mod", "Dependency Mod");
    let project_dependency_project =
        project_json("project-dep", "project-dependency", "Project Dependency");
    let router = Router::new()
        .route(
            "/v2/project/primary-project",
            get(move || {
                let payload = primary_project.clone();
                async move { Json(payload) }
            }),
        )
        .route(
            "/v2/project/dep-project",
            get(move || {
                let payload = dependency_project.clone();
                async move { Json(payload) }
            }),
        )
        .route(
            "/v2/project/project-dep",
            get(move || {
                let payload = project_dependency_project.clone();
                async move { Json(payload) }
            }),
        )
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
        replace: None,
        skip_version_ids: Vec::new(),
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
async fn resolved_batch_respects_satisfied_dependency_versions() {
    let primary = b"primary mod".to_vec();
    let dependency = b"dependency mod".to_vec();
    let project_dependency = b"project dependency".to_vec();
    let (base, server) = graph_server(primary.clone(), dependency, project_dependency).await;
    let temporary = TempDir::new().unwrap();
    let instance = instance(&temporary.path().join(".minecraft"));
    let mut request = graph_request(instance, &base, &primary);
    request.skip_version_ids = vec!["dep-v".to_owned()];

    let requests = resolve_content_install_batch(
        &graph_client(base.clone()),
        &request,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.file.filename != "dep.jar")
    );
    server.abort();
}

#[tokio::test]
async fn preflight_detects_an_installed_dependency_and_skips_it() {
    let primary = b"primary mod".to_vec();
    let dependency = b"dependency mod".to_vec();
    let project_dependency = b"project dependency".to_vec();
    let (base, server) = graph_server(primary.clone(), dependency, project_dependency).await;
    let temporary = TempDir::new().unwrap();
    let instance = instance(&temporary.path().join(".minecraft"));
    write_fabric_mod(
        &instance.mods_dir().join("dependency-mod.jar"),
        "dependency-mod",
        "1.0",
    );
    let installed = scan_installed_mods(&instance);
    let request = graph_request(instance, &base, &primary);

    let preflight = preflight_content_install(
        &graph_client(base.clone()),
        &request,
        &installed,
        None,
        &CancellationToken::new(),
    )
    .await
    .unwrap();

    let dependency = preflight
        .dependencies
        .iter()
        .find(|item| item.version_id == "dep-v")
        .unwrap();
    assert!(matches!(
        dependency.status,
        rmcl_app::ContentPreflightStatus::AlreadyInstalled { enabled: true }
    ));
    assert!(preflight.skip_version_ids.iter().any(|id| id == "dep-v"));
    assert_eq!(preflight.optional_count, 1);
    server.abort();
}

#[tokio::test]
async fn preflight_uses_remote_metadata_ranges_to_skip_a_newer_installed_dependency() {
    let primary =
        fabric_mod_bytes_with_depends("primary-mod", "1.0", json!({"dependency-mod": ">=0.5.0"}));
    let dependency = b"dependency mod".to_vec();
    let project_dependency = b"project dependency".to_vec();
    let (base, server) = graph_server(primary.clone(), dependency, project_dependency).await;
    let temporary = TempDir::new().unwrap();
    let instance = instance(&temporary.path().join(".minecraft"));
    write_fabric_mod(
        &instance.mods_dir().join("dependency-mod.jar"),
        "dependency-mod",
        "2.0",
    );
    let installed = scan_installed_mods(&instance);
    let cache = ModMetadataCache::new(
        DownloadConfig::default(),
        temporary.path().join("metadata-cache"),
    )
    .unwrap();
    let request = graph_request(instance, &base, &primary);

    let preflight = preflight_content_install(
        &graph_client(base.clone()),
        &request,
        &installed,
        Some(&cache),
        &CancellationToken::new(),
    )
    .await
    .unwrap();

    let dependency = preflight
        .dependencies
        .iter()
        .find(|item| item.version_id == "dep-v")
        .unwrap();
    assert!(matches!(
        dependency.status,
        rmcl_app::ContentPreflightStatus::AlreadyInstalled { enabled: true }
    ));
    assert!(preflight.skip_version_ids.iter().any(|id| id == "dep-v"));
    server.abort();
}

#[tokio::test]
async fn preflight_applies_remote_metadata_version_ranges_to_conflicts() {
    let primary =
        fabric_mod_bytes_with_breaks("primary-mod", "1.0", json!({"fabric-api": "<0.145.1"}));
    let (base, server) =
        graph_server(primary.clone(), b"dependency mod".to_vec(), Vec::new()).await;
    let temporary = TempDir::new().unwrap();
    let instance = instance(&temporary.path().join(".minecraft"));
    let cache = ModMetadataCache::new(
        DownloadConfig::default(),
        temporary.path().join("metadata-cache"),
    )
    .unwrap();
    let client = graph_client(base.clone());
    let request = graph_request(instance.clone(), &base, &primary);

    for (installed_version, expected_conflicts) in [
        ("0.165.0+26.3", 0),
        ("0.145.1+26.3", 0),
        ("0.144.0+26.3", 1),
    ] {
        write_fabric_mod(
            &instance.mods_dir().join("fabric-api.jar"),
            "fabric-api",
            installed_version,
        );
        let installed = scan_installed_mods(&instance);
        let preflight = preflight_content_install(
            &client,
            &request,
            &installed,
            Some(&cache),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        assert_eq!(
            preflight.incompatible.len(),
            expected_conflicts,
            "unexpected conflict result for Fabric API {installed_version}"
        );
    }
    server.abort();
}

#[tokio::test]
async fn metadata_cache_fetches_and_parses_a_remote_jar_once() {
    let bytes = fabric_mod_bytes("sodium", "0.6.0");
    let (base, server) = mod_server(bytes.clone()).await;
    let cache_dir = TempDir::new().unwrap();
    let cache = ModMetadataCache::new(DownloadConfig::default(), cache_dir.path()).unwrap();
    let file = RemoteFile {
        filename: "remote.jar".to_owned(),
        url: base.join("mod.jar").unwrap(),
        size: Some(bytes.len() as u64),
        sha1: Some(sha1(&bytes)),
        primary: true,
    };

    let first = cache.fetch(&file, &CancellationToken::new()).await.unwrap();
    assert_eq!(first.metadata.unwrap()[0].id, "sodium");
    assert!(
        std::fs::read_dir(cache_dir.path())
            .unwrap()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().ends_with(".jar"))
    );
    let second = cache.fetch(&file, &CancellationToken::new()).await.unwrap();
    assert_eq!(second.metadata.unwrap()[0].id, "sodium");
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
        replace: None,
        skip_version_ids: Vec::new(),
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
#[ignore = "downloads Iris and Sodium from the live Modrinth API"]
async fn real_modrinth_required_dependency_chain_installs() {
    let client = ModrinthClient::official().unwrap();
    let version = client
        .version("A99vfwJc", &CancellationToken::new())
        .await
        .unwrap();
    let temporary = TempDir::new().unwrap();
    let instance = instance(&temporary.path().join(".minecraft"));
    let request = ContentGraphInstallRequest {
        instance: instance.clone(),
        kind: ContentKind::Mod,
        version,
        display_name: "Iris 1.8.7".to_owned(),
        conflict: ConflictPolicy::Rename,
        game_version: "1.21.1".to_owned(),
        loader: Some("fabric".to_owned()),
        replace: None,
        skip_version_ids: Vec::new(),
    };

    let requests = resolve_content_install_batch(&client, &request, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0]
            .file
            .filename
            .to_ascii_lowercase()
            .starts_with("sodium")
    );
    assert!(
        requests[1]
            .file
            .filename
            .to_ascii_lowercase()
            .starts_with("iris")
    );

    let installer = ContentInstaller::official(DownloadConfig::default()).unwrap();
    let outcome = installer
        .install_batch(requests, CancellationToken::new(), None)
        .await
        .unwrap();
    assert_eq!(outcome.items.len(), 2);
    let mods = instance.mods_dir();
    assert!(
        std::fs::read_dir(&mods)
            .unwrap()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("sodium"))
    );
    assert!(
        std::fs::read_dir(&mods)
            .unwrap()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("iris"))
    );
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
            replace: None,
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
