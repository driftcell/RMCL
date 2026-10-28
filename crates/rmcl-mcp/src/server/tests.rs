use std::{
    io::{Cursor, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{Json, Router, routing::get};
use rmcl_app::{
    ContentInstaller, JavaService, JavaServiceConfig, LoaderCatalog, LoaderCatalogConfig,
    ModrinthClient, ModrinthConfig, TaskManager, VanillaInstaller, VanillaSources, VersionCatalog,
};
use rmcl_net::DownloadConfig;
use rmcl_store::{ConfigStore, LauncherConfig};
#[cfg(unix)]
use rmcp::{
    ClientHandler, ServiceExt,
    model::{CallToolRequestParams, ClientConfig, CustomNotification, ProtocolVersion},
};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use tempfile::TempDir;
use url::Url;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use super::*;

fn structured(result: CallToolResult) -> Value {
    result.structured_content.unwrap()
}

#[test]
fn every_tool_declares_an_output_schema() {
    let tools = McpServer::tool_router().list_all();
    assert!(!tools.is_empty());
    for tool in tools {
        assert!(
            tool.output_schema.is_some(),
            "tool {} has no output schema",
            tool.name
        );
        let annotations = tool.annotations.as_ref().unwrap();
        match tool.name.as_ref() {
            "capabilities" | "instances_list" | "instances_get" | "accounts_list"
            | "content_list" | "content_search" | "content_versions" | "versions_list"
            | "loaders_list" | "java_list" | "servers_list" | "servers_ping" | "skins_list"
            | "tasks_list" | "tasks_get" | "tasks_logs" | "tasks_unsubscribe" => {
                assert_eq!(annotations.read_only_hint, Some(true));
                assert_eq!(annotations.destructive_hint, Some(false));
                assert_eq!(annotations.idempotent_hint, Some(true));
            }
            "tasks_subscribe" => {
                assert_eq!(annotations.read_only_hint, Some(true));
                assert_eq!(annotations.destructive_hint, Some(false));
                assert_eq!(annotations.idempotent_hint, Some(false));
            }
            "servers_upsert" | "skins_import" => {
                assert_eq!(annotations.read_only_hint, Some(false));
                assert_eq!(annotations.destructive_hint, Some(false));
                assert_eq!(annotations.idempotent_hint, Some(false));
            }
            "content_delete" | "tasks_cancel" | "instances_delete" | "accounts_delete"
            | "servers_delete" | "skins_delete" => {
                assert_eq!(annotations.read_only_hint, Some(false));
                assert_eq!(annotations.destructive_hint, Some(true));
            }
            _ => {
                assert_eq!(annotations.read_only_hint, Some(false));
                assert_eq!(annotations.destructive_hint, Some(false));
            }
        }
    }
}

fn write_instance(root: &Path, id: &str) {
    let directory = root.join("versions").join(id);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join(format!("{id}.json")),
        json!({
            "id": id,
            "type": "release",
            "releaseTime": "2026-01-01T00:00:00Z",
        })
        .to_string(),
    )
    .unwrap();
}

fn write_skin(path: &Path, width: u32, height: u32) {
    let image = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    std::fs::write(path, bytes.into_inner()).unwrap();
}

fn test_context(temporary: &TempDir, modrinth: ModrinthClient) -> Arc<AppContext> {
    let minecraft_root = temporary.path().join("minecraft");
    let config = LauncherConfig {
        minecraft_root: minecraft_root.clone(),
        java_store: temporary.path().join("java"),
        auto_install_java: false,
        ..LauncherConfig::default()
    };
    let config_store = ConfigStore::new(temporary.path().join("config.toml"));
    let context = AppContext::new(&config, config_store).unwrap();
    let mut context = match Arc::try_unwrap(context) {
        Ok(context) => context,
        Err(_) => panic!("new app context should have one owner"),
    };
    let installer = VanillaInstaller::official(DownloadConfig::default()).unwrap();
    let content = ContentInstaller::official(DownloadConfig::default()).unwrap();
    context.tasks =
        TaskManager::with_content_and_modrinth(installer, content, modrinth.clone(), 32);
    context.modrinth = modrinth;
    Arc::new(context)
}

#[cfg(unix)]
fn java_test_context(temporary: &TempDir) -> Arc<AppContext> {
    let managed = temporary.path().join("managed");
    let executable = managed.join("bin/java");
    std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
    let os = if cfg!(target_os = "macos") {
        "Mac OS X"
    } else {
        "Linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "amd64"
    };
    std::fs::write(
        &executable,
        format!(
            "#!/bin/sh\nprintf 'java.version = 21.0.7\\nos.name = {os}\\nos.arch = {arch}\\njava.vendor = TestVendor\\n' >&2\n"
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();

    let context = test_context(temporary, ModrinthClient::official().unwrap());
    let mut owned_context = match Arc::try_unwrap(context) {
        Ok(context) => context,
        Err(_) => panic!("test context should have one owner"),
    };
    let mut config = JavaServiceConfig::new(temporary.path().join("java"));
    config.managed_roots.push(managed);
    config.include_java_home = false;
    config.include_path = false;
    config.auto_install = false;
    let java = JavaService::new(config).unwrap();
    owned_context.java = java.clone();
    owned_context.tasks = TaskManager::with_java(
        VanillaInstaller::official(DownloadConfig::default()).unwrap(),
        java,
        32,
    );
    Arc::new(owned_context)
}

fn shutdown_context(context: Arc<AppContext>) {
    match Arc::try_unwrap(context) {
        Ok(context) => context.runtime.shutdown_background(),
        Err(_) => panic!("app context should have no remaining owners"),
    }
}

fn push_varint(output: &mut Vec<u8>, value: i32) {
    let mut value = value as u32;
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn status_packet(status: Value) -> Vec<u8> {
    let json = status.to_string();
    let mut payload = vec![0];
    push_varint(&mut payload, json.len() as i32);
    payload.extend_from_slice(json.as_bytes());
    let mut packet = Vec::new();
    push_varint(&mut packet, payload.len() as i32);
    packet.extend_from_slice(&payload);
    packet
}

async fn read_varint_async(stream: &mut tokio::net::TcpStream) -> std::io::Result<i32> {
    use tokio::io::AsyncReadExt;

    let mut value = 0i32;
    for index in 0..5 {
        let byte = stream.read_u8().await?;
        value |= i32::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "status packet VarInt is too long",
    ))
}

async fn read_test_packet(stream: &mut tokio::net::TcpStream) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;

    let length = read_varint_async(stream).await?;
    let mut packet = vec![0; length as usize];
    stream.read_exact(&mut packet).await?;
    Ok(packet)
}

async fn ping_server() -> (String, tokio::task::JoinHandle<()>) {
    use tokio::io::AsyncWriteExt;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let handshake = read_test_packet(&mut stream).await.unwrap();
        assert_eq!(handshake[0], 0);
        let request = read_test_packet(&mut stream).await.unwrap();
        assert_eq!(request, vec![0]);
        let response = status_packet(json!({
            "version": {"name": "1.21.1", "protocol": 767},
            "players": {"online": 3, "max": 20},
            "description": {"text": "Test ", "extra": [{"text": "Server"}]},
        }));
        stream.write_all(&response).await.unwrap();
        stream.shutdown().await.unwrap();
    });
    (address, handle)
}

async fn catalog_server() -> (Url, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let manifest = json!({
        "latest": {"release": "1.21.1", "snapshot": "24w01a"},
        "versions": [{
            "id": "1.21.1",
            "type": "release",
            "url": "https://example.invalid/1.21.1.json",
            "time": "2024-08-08T08:00:00Z",
            "releaseTime": "2024-08-08T08:00:00Z"
        }]
    });
    let loaders = json!([{
        "loader": {"version": "0.16.5", "stable": true}
    }]);
    let router = Router::new()
        .route(
            "/manifest.json",
            get(move || {
                let payload = manifest.clone();
                async move { Json(payload) }
            }),
        )
        .route(
            "/v2/versions/loader/1.21.1",
            get(move || {
                let payload = loaders.clone();
                async move { Json(payload) }
            }),
        );
    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (base, handle)
}

fn install_test_catalogs(context: Arc<AppContext>, base: Url) -> Arc<AppContext> {
    let mut context = match Arc::try_unwrap(context) {
        Ok(context) => context,
        Err(_) => panic!("test context should have one owner"),
    };
    context.catalog = VersionCatalog::new(
        DownloadConfig::default(),
        VanillaSources {
            manifest_url: base.join("manifest.json").unwrap(),
            asset_object_base: base.join("objects/").unwrap(),
        },
    )
    .unwrap();
    context.loaders = LoaderCatalog::new(LoaderCatalogConfig {
        fabric_base: base.clone(),
        quilt_base: base.clone(),
        forge_metadata_base: base.clone(),
        forge_maven_base: base.clone(),
        neoforge_api_base: base.clone(),
        neoforge_maven_base: base,
        user_agent: "RMCL-test".to_owned(),
        timeout: Duration::from_secs(5),
    })
    .unwrap();
    Arc::new(context)
}

fn fabric_mod(id: &str, version: &str) -> Vec<u8> {
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    archive.start_file("fabric.mod.json", options).unwrap();
    archive
        .write_all(
            json!({"id": id, "version": version, "name": id})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    archive.finish().unwrap().into_inner()
}

fn sha1_text(bytes: &[u8]) -> String {
    hex::encode(Sha1::digest(bytes))
}

fn version_json(
    id: &str,
    project_id: &str,
    filename: &str,
    bytes: &[u8],
    url: Url,
    dependencies: Value,
) -> Value {
    json!({
        "id": id,
        "project_id": project_id,
        "name": format!("{project_id} 1.0.0"),
        "version_number": "1.0.0",
        "version_type": "release",
        "date_published": "2026-01-01T00:00:00Z",
        "game_versions": ["1.21.1"],
        "loaders": ["fabric"],
        "files": [{
            "hashes": {"sha1": sha1_text(bytes)},
            "url": url.to_string(),
            "filename": filename,
            "primary": true,
            "size": bytes.len(),
        }],
        "dependencies": dependencies,
    })
}

fn project_json(id: &str, slug: &str) -> Value {
    json!({
        "id": id,
        "slug": slug,
        "title": slug,
        "description": format!("{slug} test project"),
        "categories": ["fabric"],
        "project_type": "mod",
        "downloads": 10,
        "follows": 2,
        "game_versions": ["1.21.1"],
        "loaders": ["fabric"],
        "updated": "2026-01-01T00:00:00Z",
    })
}

async fn remote_server(main: Vec<u8>, dependency: Vec<u8>) -> (Url, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let main_version = version_json(
        "main-version",
        "main-project",
        "main.jar",
        &main,
        base.join("files/main.jar").unwrap(),
        json!([{"version_id": "dependency-version", "dependency_type": "required"}]),
    );
    let conflict_version = version_json(
        "conflict-version",
        "main-project",
        "main.jar",
        &main,
        base.join("files/main.jar").unwrap(),
        json!([{"project_id": "dependency-project", "dependency_type": "incompatible"}]),
    );
    let dependency_version = version_json(
        "dependency-version",
        "dependency-project",
        "dependency.jar",
        &dependency,
        base.join("files/dependency.jar").unwrap(),
        json!([]),
    );
    let search = json!({
        "hits": [{
            "project_id": "main-project",
            "project_type": "mod",
            "slug": "main-mod",
            "author": "RMCL",
            "title": "main-mod",
            "description": "Main test project",
            "categories": ["fabric"],
            "versions": ["1.21.1"],
            "downloads": 10,
            "follows": 2,
            "date_modified": "2026-01-01T00:00:00Z",
        }],
        "offset": 0,
        "limit": 20,
        "total_hits": 1,
    });
    let main_project = project_json("main-project", "main-mod");
    let dependency_project = project_json("dependency-project", "dependency-mod");
    let versions = vec![main_version.clone(), conflict_version.clone()];
    let router = Router::new()
        .route(
            "/v2/search",
            get(move || {
                let payload = search.clone();
                async move { Json(payload) }
            }),
        )
        .route(
            "/v2/project/main-project",
            get(move || {
                let payload = main_project.clone();
                async move { Json(payload) }
            }),
        )
        .route(
            "/v2/project/dependency-project",
            get(move || {
                let payload = dependency_project.clone();
                async move { Json(payload) }
            }),
        )
        .route(
            "/v2/project/main-project/version",
            get(move || {
                let payload = versions.clone();
                async move { Json(payload) }
            }),
        )
        .route(
            "/v2/version/main-version",
            get(move || {
                let payload = main_version.clone();
                async move { Json(payload) }
            }),
        )
        .route(
            "/v2/version/conflict-version",
            get(move || {
                let payload = conflict_version.clone();
                async move { Json(payload) }
            }),
        )
        .route(
            "/v2/version/dependency-version",
            get(move || {
                let payload = dependency_version.clone();
                async move { Json(payload) }
            }),
        )
        .route(
            "/files/main.jar",
            get(move || {
                let bytes = main.clone();
                async move { bytes }
            }),
        )
        .route(
            "/files/dependency.jar",
            get(move || {
                let bytes = dependency.clone();
                async move { bytes }
            }),
        );
    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (base, handle)
}

fn install_params(
    version_id: &str,
    revision: &str,
    key: &str,
    dry_run: bool,
) -> ContentInstallParams {
    ContentInstallParams {
        instance_id: "example".to_owned(),
        kind: "mod".to_owned(),
        version_id: version_id.to_owned(),
        conflict: Some("reject".to_owned()),
        expected_revision: Some(revision.to_owned()),
        idempotency_key: Some(key.to_owned()),
        dry_run: Some(dry_run),
    }
}

#[test]
fn local_content_tools_enforce_revision_and_create_dry_run() {
    let temporary = TempDir::new().unwrap();
    write_instance(&temporary.path().join("minecraft"), "example");
    let context = test_context(&temporary, ModrinthClient::official().unwrap());
    let (catalog_base, catalog_handle) = context.runtime.block_on(catalog_server());
    let context = install_test_catalogs(context, catalog_base);
    let server = McpServer::new(context.clone());
    context.runtime.block_on(async {
        let instance = structured(
            server
                .instances_get(Parameters(InstanceIdParams {
                    instance_id: "example".to_owned(),
                }))
                .await
                .unwrap(),
        );
        let revision = instance["revision"].as_str().unwrap();
        let source = temporary.path().join("source.jar");
        std::fs::write(&source, fabric_mod("local-mod", "1.0.0")).unwrap();
        let imported = structured(
            server
                .content_import(Parameters(ContentImportParams {
                    instance_id: "example".to_owned(),
                    kind: "mod".to_owned(),
                    source: source.display().to_string(),
                    conflict: Some("reject".to_owned()),
                    expected_revision: Some(revision.to_owned()),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(imported["content"]["relative_path"], "source.jar");

        let repository_guard = try_lock_repository(temporary.path().join("minecraft")).unwrap();
        let blocked = server
            .content_import(Parameters(ContentImportParams {
                instance_id: "example".to_owned(),
                kind: "mod".to_owned(),
                source: source.display().to_string(),
                conflict: Some("reject".to_owned()),
                expected_revision: Some(revision.to_owned()),
            }))
            .await
            .unwrap_err();
        assert_eq!(blocked.data.as_ref().unwrap()["code"], "repository_busy");
        drop(repository_guard);

        let stale = server
            .content_delete(Parameters(ContentDeleteParams {
                instance_id: "example".to_owned(),
                kind: "mod".to_owned(),
                relative_path: "source.jar".to_owned(),
                expected_revision: "sha256:stale".to_owned(),
            }))
            .await
            .unwrap_err();
        assert!(stale.message.contains("revision conflict"));
        assert!(temporary.path().join("minecraft/mods/source.jar").is_file());

        let disabled = structured(
            server
                .content_set_enabled(Parameters(ContentSetEnabledParams {
                    instance_id: "example".to_owned(),
                    kind: "mod".to_owned(),
                    relative_path: "source.jar".to_owned(),
                    enabled: false,
                    expected_revision: Some(revision.to_owned()),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(disabled["content"]["enabled"], false);
        assert_eq!(disabled["content"]["relative_path"], "source.jar.disabled");

        let enabled = structured(
            server
                .content_set_enabled(Parameters(ContentSetEnabledParams {
                    instance_id: "example".to_owned(),
                    kind: "mod".to_owned(),
                    relative_path: "source.jar.disabled".to_owned(),
                    enabled: true,
                    expected_revision: Some(revision.to_owned()),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(enabled["content"]["enabled"], true);
        let deleted = structured(
            server
                .content_delete(Parameters(ContentDeleteParams {
                    instance_id: "example".to_owned(),
                    kind: "mod".to_owned(),
                    relative_path: "source.jar".to_owned(),
                    expected_revision: revision.to_owned(),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(deleted["deleted"], true);

        let versions = structured(
            server
                .versions_list(Parameters(VersionListParams {
                    query: Some("1.21".to_owned()),
                    version_type: Some("release".to_owned()),
                    limit: Some(10),
                    refresh: Some(true),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(versions["latest_release"], "1.21.1");
        assert_eq!(versions["items"].as_array().unwrap().len(), 1);
        let loaders = structured(
            server
                .loaders_list(Parameters(LoaderVersionsParams {
                    kind: "fabric".to_owned(),
                    game_version: "1.21.1".to_owned(),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(loaders["items"][0]["version"], "0.16.5");
        assert_eq!(loaders["items"][0]["stable"], true);

        let plan = structured(
            server
                .instances_create(Parameters(CreateInstanceParams {
                    instance_id: "dry-run".to_owned(),
                    base_version_id: "1.21.1".to_owned(),
                    name: None,
                    loader: Some(LoaderParams {
                        kind: "fabric".to_owned(),
                        version: "0.16.5".to_owned(),
                        installer_version: None,
                    }),
                    copy_content_from: None,
                    idempotency_key: Some("dry-run".to_owned()),
                    dry_run: Some(true),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(plan["dry_run"], true);
        assert!(plan["checks"].as_array().unwrap().len() >= 3);
        assert!(!temporary.path().join("minecraft/versions/dry-run").exists());
        let tasks = structured(server.tasks_list().await.unwrap());
        assert_eq!(tasks["items"].as_array().unwrap().len(), 0);

        let delete_plan = structured(
            server
                .instances_delete(Parameters(DeleteInstanceParams {
                    instance_id: "example".to_owned(),
                    expected_revision: revision.to_owned(),
                    dry_run: Some(true),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(delete_plan["dry_run"], true);
        assert_eq!(delete_plan["deletes_isolated_content"], false);
        let stale_delete = server
            .instances_delete(Parameters(DeleteInstanceParams {
                instance_id: "example".to_owned(),
                expected_revision: "sha256:stale".to_owned(),
                dry_run: None,
            }))
            .await
            .unwrap_err();
        assert!(stale_delete.message.contains("revision conflict"));
        assert!(temporary.path().join("minecraft/versions/example").exists());
        let deleted_instance = structured(
            server
                .instances_delete(Parameters(DeleteInstanceParams {
                    instance_id: "example".to_owned(),
                    expected_revision: revision.to_owned(),
                    dry_run: Some(false),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(deleted_instance["deleted"], true);
        assert!(!temporary.path().join("minecraft/versions/example").exists());
    });
    drop(server);
    catalog_handle.abort();
    shutdown_context(context);
}

#[test]
fn read_only_access_allows_queries_and_dry_run_but_rejects_writes() {
    let temporary = TempDir::new().unwrap();
    write_instance(&temporary.path().join("minecraft"), "example");
    let context = test_context(&temporary, ModrinthClient::official().unwrap());
    let (catalog_base, catalog_handle) = context.runtime.block_on(catalog_server());
    let context = install_test_catalogs(context, catalog_base);
    let server = McpServer::with_access(context.clone(), AccessLevel::ReadOnly);
    context.runtime.block_on(async {
        let capabilities = structured(server.capabilities().await.unwrap());
        assert_eq!(capabilities["access"], "read_only");
        let versions = structured(
            server
                .versions_list(Parameters(VersionListParams {
                    query: Some("1.21".to_owned()),
                    version_type: Some("release".to_owned()),
                    limit: Some(10),
                    refresh: Some(true),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(versions["latest_release"], "1.21.1");
        assert_eq!(versions["items"].as_array().unwrap().len(), 1);
        let loaders = structured(
            server
                .loaders_list(Parameters(LoaderVersionsParams {
                    kind: "fabric".to_owned(),
                    game_version: "1.21.1".to_owned(),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(loaders["items"][0]["version"], "0.16.5");
        assert_eq!(loaders["items"][0]["stable"], true);

        let plan = structured(
            server
                .instances_create(Parameters(CreateInstanceParams {
                    instance_id: "dry-run".to_owned(),
                    base_version_id: "1.21.1".to_owned(),
                    name: None,
                    loader: Some(LoaderParams {
                        kind: "fabric".to_owned(),
                        version: "0.16.5".to_owned(),
                        installer_version: None,
                    }),
                    copy_content_from: None,
                    idempotency_key: None,
                    dry_run: Some(true),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(plan["dry_run"], true);
        assert!(plan["checks"].as_array().unwrap().len() >= 3);

        let create = server
            .instances_create(Parameters(CreateInstanceParams {
                instance_id: "blocked".to_owned(),
                base_version_id: "missing".to_owned(),
                name: None,
                loader: None,
                copy_content_from: None,
                idempotency_key: None,
                dry_run: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(create.data.as_ref().unwrap()["code"], "permission_denied");
        let update = server
            .instances_update(Parameters(UpdateInstanceParams {
                instance_id: "example".to_owned(),
                name: Some("Blocked".to_owned()),
                java: None,
                max_memory_mb: None,
                window: None,
                clear_window: None,
                isolation: None,
                expected_revision: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(update.data.as_ref().unwrap()["code"], "permission_denied");
        let delete = server
            .content_delete(Parameters(ContentDeleteParams {
                instance_id: "example".to_owned(),
                kind: "mod".to_owned(),
                relative_path: "missing.jar".to_owned(),
                expected_revision: "sha256:any".to_owned(),
            }))
            .await
            .unwrap_err();
        assert_eq!(delete.data.as_ref().unwrap()["code"], "permission_denied");
        let server_write = server
            .servers_upsert(Parameters(ServerUpsertParams {
                server_id: None,
                name: "Blocked".to_owned(),
                address: "localhost".to_owned(),
            }))
            .await
            .unwrap_err();
        assert_eq!(
            server_write.data.as_ref().unwrap()["code"],
            "permission_denied"
        );
        let skin_write = server
            .skins_import(Parameters(SkinImportParams {
                source: "does-not-exist.png".to_owned(),
                name: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(
            skin_write.data.as_ref().unwrap()["code"],
            "permission_denied"
        );
        assert!(!temporary.path().join("minecraft/versions/blocked").exists());
        assert_eq!(
            structured(server.tasks_list().await.unwrap())["items"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    });
    drop(server);
    catalog_handle.abort();
    shutdown_context(context);
}

#[test]
fn server_bookmarks_and_ping_use_shared_services() {
    let temporary = TempDir::new().unwrap();
    let context = test_context(&temporary, ModrinthClient::official().unwrap());
    let (address, ping_handle) = context.runtime.block_on(ping_server());
    let server = McpServer::new(context.clone());
    context.runtime.block_on(async {
        assert!(
            structured(server.servers_list().await.unwrap())["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let created = structured(
            server
                .servers_upsert(Parameters(ServerUpsertParams {
                    server_id: None,
                    name: String::new(),
                    address: address.clone(),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(created["created"], true);
        assert_eq!(created["server"]["name"], address);
        let server_id = created["server"]["id"].as_str().unwrap().to_owned();

        let ping = structured(
            server
                .servers_ping(Parameters(ServerPingParams {
                    server_id: Some(server_id.clone()),
                    address: None,
                }))
                .await
                .unwrap(),
        );
        assert_eq!(ping["server_id"], server_id);
        assert_eq!(ping["motd"], "Test Server");
        assert_eq!(ping["online_players"], 3);
        assert_eq!(ping["max_players"], 20);
        assert_eq!(ping["version_name"], "1.21.1");
        assert_eq!(ping["protocol"], 767);
        assert!(ping["latency_ms"].is_number());
        assert_eq!(ping["icon_path"], Value::Null);
        ping_handle.await.unwrap();

        let updated = structured(
            server
                .servers_upsert(Parameters(ServerUpsertParams {
                    server_id: Some(server_id.clone()),
                    name: "Renamed".to_owned(),
                    address: address.clone(),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(updated["created"], false);
        assert_eq!(updated["server"]["name"], "Renamed");
        let invalid = server
            .servers_upsert(Parameters(ServerUpsertParams {
                server_id: None,
                name: "Invalid".to_owned(),
                address: "https://example.com".to_owned(),
            }))
            .await
            .unwrap_err();
        assert_eq!(invalid.data.as_ref().unwrap()["code"], "invalid_input");

        let deleted = structured(
            server
                .servers_delete(Parameters(ServerIdParams {
                    server_id: server_id.clone(),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(deleted["deleted"], true);
        assert!(
            structured(server.servers_list().await.unwrap())["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let missing = server
            .servers_delete(Parameters(ServerIdParams { server_id }))
            .await
            .unwrap_err();
        assert_eq!(missing.data.as_ref().unwrap()["code"], "not_found");
    });
    drop(server);
    shutdown_context(context);
}

#[test]
fn skin_library_tools_validate_import_rename_and_delete() {
    let temporary = TempDir::new().unwrap();
    let context = test_context(&temporary, ModrinthClient::official().unwrap());
    let server = McpServer::new(context.clone());
    let source = temporary.path().join("source.png");
    write_skin(&source, 64, 64);
    context.runtime.block_on(async {
        let imported = structured(
            server
                .skins_import(Parameters(SkinImportParams {
                    source: source.display().to_string(),
                    name: Some("My Skin".to_owned()),
                }))
                .await
                .unwrap(),
        );
        let skin_id = imported["skin"]["id"].as_str().unwrap().to_owned();
        assert_eq!(imported["skin"]["name"], "My Skin");
        assert_eq!(imported["skin"]["variant"], "classic");
        assert!(Path::new(imported["skin"]["head_path"].as_str().unwrap()).is_file());

        let duplicate = structured(
            server
                .skins_import(Parameters(SkinImportParams {
                    source: source.display().to_string(),
                    name: None,
                }))
                .await
                .unwrap(),
        );
        assert_eq!(duplicate["skin"]["id"], skin_id);
        assert_eq!(duplicate["skin"]["name"], "source.png");
        assert_eq!(
            structured(server.skins_list().await.unwrap())["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        let invalid = temporary.path().join("invalid.png");
        write_skin(&invalid, 1, 1);
        let invalid_error = server
            .skins_import(Parameters(SkinImportParams {
                source: invalid.display().to_string(),
                name: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(
            invalid_error.data.as_ref().unwrap()["code"],
            "invalid_input"
        );
        let private_url = server
            .skins_import(Parameters(SkinImportParams {
                source: "https://127.0.0.1/skin.png".to_owned(),
                name: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(private_url.data.as_ref().unwrap()["code"], "invalid_input");

        let renamed = structured(
            server
                .skins_rename(Parameters(SkinRenameParams {
                    skin_id: skin_id.clone(),
                    name: "Renamed".to_owned(),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(renamed["skin"]["id"], skin_id);
        assert_eq!(renamed["skin"]["name"], "Renamed");

        let deleted = structured(
            server
                .skins_delete(Parameters(SkinIdParams {
                    skin_id: skin_id.clone(),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(deleted["deleted"], true);
        assert!(
            structured(server.skins_list().await.unwrap())["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let missing = server
            .skins_delete(Parameters(SkinIdParams { skin_id }))
            .await
            .unwrap_err();
        assert_eq!(missing.data.as_ref().unwrap()["code"], "not_found");
    });
    drop(server);
    shutdown_context(context);
}

#[test]
fn account_tools_create_select_launch_and_delete_offline_accounts() {
    let temporary = TempDir::new().unwrap();
    let context = test_context(&temporary, ModrinthClient::official().unwrap());
    let server = McpServer::new(context.clone());
    context.runtime.block_on(async {
        let alex = structured(
            server
                .accounts_create_offline(Parameters(CreateOfflineAccountParams {
                    username: "Alex".to_owned(),
                    uuid: Some("00000000-0000-0000-0000-000000000001".to_owned()),
                    make_current: Some(true),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(alex["account"]["selected"], true);
        let steve = structured(
            server
                .accounts_create_offline(Parameters(CreateOfflineAccountParams {
                    username: "Steve".to_owned(),
                    uuid: None,
                    make_current: Some(false),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(steve["account"]["selected"], false);
        let selected = structured(
            server
                .accounts_select(Parameters(AccountSelectorParams {
                    username: "Steve".to_owned(),
                    kind: "offline".to_owned(),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(selected["account"]["selected"], true);

        let launch = server
            .launch_account(Some(&AccountSelectorParams {
                username: "Alex".to_owned(),
                kind: "offline".to_owned(),
            }))
            .await
            .unwrap();
        assert_eq!(launch.username, "Alex");
        let accounts = structured(server.accounts_list().await.unwrap());
        assert_eq!(accounts["selected_account"], "Steve");
        assert_eq!(accounts["selected_account_kind"], "offline");
        assert_eq!(accounts["items"].as_array().unwrap().len(), 2);

        let deleted = structured(
            server
                .accounts_delete(Parameters(AccountSelectorParams {
                    username: "Steve".to_owned(),
                    kind: "offline".to_owned(),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(deleted["deleted"], true);
        let accounts = structured(server.accounts_list().await.unwrap());
        assert_eq!(accounts["selected_account"], "Alex");
        assert_eq!(accounts["items"].as_array().unwrap().len(), 1);
        let missing = server
            .accounts_delete(Parameters(AccountSelectorParams {
                username: "Steve".to_owned(),
                kind: "offline".to_owned(),
            }))
            .await
            .unwrap_err();
        assert_eq!(missing.data.as_ref().unwrap()["code"], "not_found");
    });
    drop(server);
    shutdown_context(context);
}

#[cfg(unix)]
#[test]
fn java_ensure_task_uses_a_managed_runtime_and_completes() {
    let temporary = TempDir::new().unwrap();
    let context = java_test_context(&temporary);
    let server = McpServer::new(context.clone());
    context.runtime.block_on(async {
        let started = structured(
            server
                .java_ensure(Parameters(JavaEnsureParams {
                    major_version: Some(21),
                    component: None,
                }))
                .await
                .unwrap(),
        );
        let task_id = started["task_id"].as_u64().unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let task = structured(
                    server
                        .tasks_get(Parameters(TaskIdParams { task_id }))
                        .await
                        .unwrap(),
                );
                match task["status"].as_str().unwrap() {
                    "completed" => {
                        assert_eq!(task["kind"], "java_install");
                        assert_eq!(task["subject"]["type"], "java");
                        assert_eq!(task["subject"]["major_version"], 21);
                        break;
                    }
                    "failed" | "cancelled" => panic!("Java task failed: {task}"),
                    _ => tokio::time::sleep(Duration::from_millis(20)).await,
                }
            }
        })
        .await
        .unwrap();
    });
    drop(server);
    shutdown_context(context);
}

#[cfg(unix)]
#[derive(Clone, Default)]
struct NotificationCollector {
    items: Arc<Mutex<Vec<CustomNotification>>>,
}

#[cfg(unix)]
impl ClientHandler for NotificationCollector {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::default().with_protocol_version(ProtocolVersion::LATEST_WITH_INITIALIZE)
    }

    async fn on_custom_notification(
        &self,
        notification: CustomNotification,
        _context: rmcp::service::NotificationContext<rmcp::RoleClient>,
    ) {
        if notification.method == crate::notifications::TASK_NOTIFICATION_METHOD {
            self.items.lock().await.push(notification);
        }
    }
}

#[cfg(unix)]
async fn wait_for_client_task(client: &rmcp::Peer<rmcp::RoleClient>, task_id: u64) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let task =
                structured(
                    client
                        .call_tool(CallToolRequestParams::new("tasks_get").with_arguments(
                            json!({"task_id": task_id}).as_object().unwrap().clone(),
                        ))
                        .await
                        .unwrap(),
                );
            match task["status"].as_str().unwrap() {
                "completed" => return task,
                "failed" | "cancelled" => panic!("task failed: {task}"),
                _ => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
    })
    .await
    .unwrap()
}

#[cfg(unix)]
async fn wait_for_terminal_notification(collector: &NotificationCollector, task_id: u64) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let notification = collector
                .items
                .lock()
                .await
                .iter()
                .find_map(|notification| {
                    notification
                        .params
                        .as_ref()
                        .filter(|params| {
                            params["type"] == "snapshot"
                                && params["task"]["id"] == task_id
                                && params["task"]["status"] == "completed"
                        })
                        .cloned()
                });
            if let Some(params) = notification {
                return params;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}

#[cfg(unix)]
async fn start_java_via_client(client: &rmcp::Peer<rmcp::RoleClient>) -> u64 {
    let started = structured(
        client
            .call_tool(
                CallToolRequestParams::new("java_ensure")
                    .with_arguments(json!({"major_version": 21}).as_object().unwrap().clone()),
            )
            .await
            .unwrap(),
    );
    started["task_id"].as_u64().unwrap()
}

#[cfg(unix)]
#[test]
fn task_notifications_require_subscription_and_stop_after_unsubscribe() {
    let temporary = TempDir::new().unwrap();
    let context = java_test_context(&temporary);
    context.runtime.block_on(async {
        let (server_transport, client_transport) = tokio::io::duplex(64 * 1024);
        let server = McpServer::new(context.clone());
        let server_handle = tokio::spawn(async move {
            let running = server.serve(server_transport).await.unwrap();
            running.waiting().await.unwrap();
        });
        let collector = NotificationCollector::default();
        let client = collector.clone().serve(client_transport).await.unwrap();

        let first = start_java_via_client(&client).await;
        wait_for_client_task(&client, first).await;
        assert!(collector.items.lock().await.is_empty());

        let subscription = structured(
            client
                .call_tool(
                    CallToolRequestParams::new("tasks_subscribe").with_arguments(
                        json!({"include_logs": false}).as_object().unwrap().clone(),
                    ),
                )
                .await
                .unwrap(),
        );
        let subscription_id = subscription["subscription_id"].as_u64().unwrap();
        assert_eq!(subscription["items"].as_array().unwrap().len(), 1);
        assert_eq!(subscription["items"][0]["id"], first);
        assert_eq!(subscription["include_logs"], false);

        let second = start_java_via_client(&client).await;
        let notification = wait_for_terminal_notification(&collector, second).await;
        assert_eq!(notification["subscription_id"], subscription_id);
        assert_eq!(notification["task"]["kind"], "java_install");
        assert!(collector.items.lock().await.iter().all(|notification| {
            notification
                .params
                .as_ref()
                .is_some_and(|params| params["type"] == "snapshot")
        }));

        let stopped = structured(
            client
                .call_tool(
                    CallToolRequestParams::new("tasks_unsubscribe").with_arguments(
                        json!({"subscription_id": subscription_id})
                            .as_object()
                            .unwrap()
                            .clone(),
                    ),
                )
                .await
                .unwrap(),
        );
        assert_eq!(stopped["unsubscribed"], true);
        let notification_count = collector.items.lock().await.len();
        let third = start_java_via_client(&client).await;
        wait_for_client_task(&client, third).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(collector.items.lock().await.len(), notification_count);

        client.cancel().await.unwrap();
        server_handle.await.unwrap();
    });
    shutdown_context(context);
}

#[test]
fn remote_content_tools_preflight_install_and_replay() {
    let temporary = TempDir::new().unwrap();
    write_instance(&temporary.path().join("minecraft"), "example");
    let context = test_context(&temporary, ModrinthClient::official().unwrap());
    let (base, handle) = context.runtime.block_on(remote_server(
        fabric_mod("main-mod", "1.0.0"),
        fabric_mod("dependency-mod", "1.0.0"),
    ));
    let client = ModrinthClient::new(ModrinthConfig::new(base)).unwrap();
    let mut owned_context = match Arc::try_unwrap(context) {
        Ok(context) => context,
        Err(_) => panic!("test context should have one owner"),
    };
    owned_context.modrinth = client.clone();
    owned_context.tasks = TaskManager::with_content_and_modrinth(
        VanillaInstaller::official(DownloadConfig::default()).unwrap(),
        ContentInstaller::official(DownloadConfig::default()).unwrap(),
        client,
        32,
    );
    let context = Arc::new(owned_context);
    let server = McpServer::new(context.clone());
    context.runtime.block_on(async {
        let search = structured(
            server
                .content_search(Parameters(ContentSearchParams {
                    kind: "mod".to_owned(),
                    query: "main".to_owned(),
                    game_version: Some("1.21.1".to_owned()),
                    loader: Some("fabric".to_owned()),
                    offset: None,
                    limit: Some(20),
                    sort: Some("relevance".to_owned()),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(search["total_hits"], 1);
        assert_eq!(search["items"][0]["id"], "main-project");
        let versions = structured(
            server
                .content_versions(Parameters(ContentVersionsParams {
                    project_id: "main-project".to_owned(),
                    game_version: Some("1.21.1".to_owned()),
                    loader: Some("fabric".to_owned()),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(versions["items"][0]["id"], "main-version");

        let instance = structured(
            server
                .instances_get(Parameters(InstanceIdParams {
                    instance_id: "example".to_owned(),
                }))
                .await
                .unwrap(),
        );
        let revision = instance["revision"].as_str().unwrap().to_owned();
        let preflight = structured(
            server
                .content_install(Parameters(install_params(
                    "main-version",
                    &revision,
                    "dry-run",
                    true,
                )))
                .await
                .unwrap(),
        );
        assert_eq!(preflight["main"]["status"]["state"], "will_install");
        assert_eq!(
            preflight["dependencies"][0]["status"]["state"],
            "will_install"
        );
        assert_eq!(
            structured(server.tasks_list().await.unwrap())["items"]
                .as_array()
                .unwrap()
                .len(),
            0
        );

        let started = structured(
            server
                .content_install(Parameters(install_params(
                    "main-version",
                    &revision,
                    "install",
                    false,
                )))
                .await
                .unwrap(),
        );
        let replay = structured(
            server
                .content_install(Parameters(install_params(
                    "main-version",
                    &revision,
                    "install",
                    false,
                )))
                .await
                .unwrap(),
        );
        assert_eq!(started["idempotent_replay"], false);
        assert_eq!(replay["idempotent_replay"], true);
        assert_eq!(replay["task_id"], started["task_id"]);
        let task_id = started["task_id"].as_u64().unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let task = structured(
                    server
                        .tasks_get(Parameters(TaskIdParams { task_id }))
                        .await
                        .unwrap(),
                );
                match task["status"].as_str().unwrap() {
                    "completed" => break,
                    "failed" | "cancelled" => panic!("content task failed: {task}"),
                    _ => tokio::time::sleep(Duration::from_millis(20)).await,
                }
            }
        })
        .await
        .unwrap();
        let contents = structured(
            server
                .content_list(Parameters(ContentListParams {
                    instance_id: "example".to_owned(),
                    kind: Some("mod".to_owned()),
                }))
                .await
                .unwrap(),
        );
        let mut names = contents["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["relative_path"].as_str().unwrap())
            .collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(names, ["dependency.jar", "main.jar"]);

        let installed = structured(
            server
                .content_install(Parameters(install_params(
                    "main-version",
                    &revision,
                    "second-dry-run",
                    true,
                )))
                .await
                .unwrap(),
        );
        assert_eq!(installed["main"]["status"]["state"], "already_installed");
        assert_eq!(
            installed["dependencies"][0]["status"]["state"],
            "already_installed"
        );
        assert_eq!(installed["skip_version_ids"], json!(["dependency-version"]));

        let conflict = server
            .content_install(Parameters(install_params(
                "conflict-version",
                &revision,
                "conflict",
                false,
            )))
            .await
            .unwrap_err();
        assert!(conflict.message.contains("incompatible installed mods"));
        assert_eq!(
            structured(server.tasks_list().await.unwrap())["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    });
    drop(server);
    handle.abort();
    shutdown_context(context);
}

#[test]
fn concurrent_idempotency_keys_start_only_one_task() {
    let temporary = TempDir::new().unwrap();
    let context = test_context(&temporary, ModrinthClient::official().unwrap());
    let server = McpServer::new(context.clone());
    context.runtime.block_on(async {
        let starts = Arc::new(AtomicUsize::new(0));
        let first_starts = starts.clone();
        let second_starts = starts.clone();
        let first = server.idempotent_task("test", Some("concurrent"), || async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            first_starts.fetch_add(1, Ordering::SeqCst);
            Ok(TaskId::from_raw(901))
        });
        let second = server.idempotent_task("test", Some("concurrent"), || async move {
            second_starts.fetch_add(1, Ordering::SeqCst);
            Ok(TaskId::from_raw(902))
        });
        let (first, second) = tokio::join!(first, second);
        let (first_id, first_replay) = first.unwrap();
        let (second_id, second_replay) = second.unwrap();
        assert_eq!(first_id, second_id);
        assert_ne!(first_replay, second_replay);
        assert_eq!(starts.load(Ordering::SeqCst), 1);
    });
    drop(server);
    shutdown_context(context);
}
