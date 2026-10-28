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
    ContentInstaller, LoaderCatalog, LoaderCatalogConfig, ModrinthClient, ModrinthConfig,
    TaskManager, VanillaInstaller, VanillaSources, VersionCatalog,
};
use rmcl_net::DownloadConfig;
use rmcl_store::{ConfigStore, LauncherConfig};
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
            "capabilities" | "instances_list" | "instances_get" | "content_list"
            | "content_search" | "content_versions" | "versions_list" | "loaders_list"
            | "java_list" | "tasks_list" | "tasks_get" | "tasks_logs" => {
                assert_eq!(annotations.read_only_hint, Some(true));
                assert_eq!(annotations.destructive_hint, Some(false));
                assert_eq!(annotations.idempotent_hint, Some(true));
            }
            "content_delete" | "tasks_cancel" | "instances_delete" => {
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

fn shutdown_context(context: Arc<AppContext>) {
    match Arc::try_unwrap(context) {
        Ok(context) => context.runtime.shutdown_background(),
        Err(_) => panic!("app context should have no remaining owners"),
    }
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
