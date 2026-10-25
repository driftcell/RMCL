use std::path::Path;

use rmcl_install::{
    InstallPlan, InstallPlanError, PlannedFile, PlannedFileKind, plan_asset_objects,
    plan_asset_objects_with_base, plan_vanilla,
};
use rmcl_instance::MinecraftLayout;
use rmcl_minecraft::{AssetCatalog, GameVersion, OperatingSystem, Platform};
use serde_json::{Value, json};

const HASH_A: &str = "0123456789abcdef0123456789abcdef01234567";
const HASH_B: &str = "abcdefabcdefabcdefabcdefabcdefabcdefabcd";

fn layout() -> MinecraftLayout {
    MinecraftLayout::new(".minecraft")
}

fn linux() -> Platform {
    Platform::new(OperatingSystem::Linux, "6.1", "x86_64")
}

fn base_version(libraries: Value) -> Value {
    json!({
        "id": "example",
        "mainClass": "example.Main",
        "downloads": {
            "client": {
                "url": "https://example.invalid/client.jar",
                "sha1": HASH_A,
                "size": 10
            }
        },
        "assetIndex": {
            "id": "example-index",
            "url": "https://example.invalid/index.json",
            "sha1": HASH_A,
            "size": 20
        },
        "libraries": libraries
    })
}

fn version(metadata: Value) -> GameVersion {
    GameVersion::from_json(&metadata.to_string()).unwrap()
}

fn plan(metadata: Value) -> InstallPlan {
    plan_vanilla(&layout(), &version(metadata), &linux()).unwrap()
}

fn files(plan: &InstallPlan, kind: PlannedFileKind) -> Vec<&PlannedFile> {
    plan.files.iter().filter(|file| file.kind == kind).collect()
}

#[test]
fn plans_modern_vanilla_and_classpath_native_library() {
    let result = plan(base_version(json!([
        {
            "name": "com.example:core:1.0",
            "downloads": {
                "artifact": {
                    "path": "custom/core.jar",
                    "url": "https://example.invalid/core.jar",
                    "sha1": HASH_A,
                    "size": 100
                }
            }
        },
        {
            "name": "org.lwjgl:lwjgl:3.3.3:natives-linux",
            "rules": [{"action": "allow", "os": {"name": "linux"}}],
            "downloads": {
                "artifact": {
                    "path": "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-linux.jar",
                    "url": "https://example.invalid/lwjgl-natives.jar",
                    "sha1": HASH_B,
                    "size": 50
                }
            }
        }
    ])));

    assert_eq!(result.files.len(), 4);
    assert_eq!(files(&result, PlannedFileKind::Library).len(), 2);
    assert!(files(&result, PlannedFileKind::NativeLibrary).is_empty());
    assert!(result.extractions.is_empty());

    let client = files(&result, PlannedFileKind::Client)[0];
    assert_eq!(
        client.destination,
        Path::new(".minecraft").join("versions/example/example.jar")
    );
    assert_eq!(client.size, Some(10));

    let core = files(&result, PlannedFileKind::Library)[0];
    assert_eq!(
        core.destination,
        Path::new(".minecraft").join("libraries/custom/core.jar")
    );
    assert_eq!(core.sources[0].as_str(), "https://example.invalid/core.jar");
    assert_eq!(core.sources.len(), 1);

    let index = files(&result, PlannedFileKind::AssetIndex)[0];
    assert_eq!(
        index.destination,
        Path::new(".minecraft").join("assets/indexes/example-index.json")
    );
}

#[test]
fn plans_old_style_maven_library_with_custom_base() {
    let result = plan(base_version(json!([{
        "name": "com.example:legacy:1.0",
        "url": "https://example.invalid/maven"
    }])));

    let library = files(&result, PlannedFileKind::Library)[0];
    assert_eq!(
        library.destination,
        Path::new(".minecraft").join("libraries/com/example/legacy/1.0/legacy-1.0.jar")
    );
    assert_eq!(
        library.sources[0].as_str(),
        "https://example.invalid/maven/com/example/legacy/1.0/legacy-1.0.jar"
    );
    assert_eq!(library.sha1, None);
    assert_eq!(library.size, None);
}

#[test]
fn skips_libraries_rejected_by_rules() {
    let result = plan(base_version(json!([{
        "name": "com.example:windows-only:1.0",
        "rules": [{"action": "allow", "os": {"name": "windows"}}]
    }])));

    assert_eq!(result.files.len(), 2);
    assert!(files(&result, PlannedFileKind::Library).is_empty());
}

#[test]
fn plans_native_classifier_and_extraction() {
    let result = plan(base_version(json!([{
        "name": "com.example:native:1.0",
        "natives": {"linux": "natives-linux-${arch}"},
        "downloads": {
            "classifiers": {
                "natives-linux-64": {
                    "path": "com/example/native/1.0/native-1.0-natives-linux-64.jar",
                    "url": "https://example.invalid/native-linux.jar",
                    "sha1": HASH_A,
                    "size": 64
                }
            }
        },
        "extract": {"exclude": ["META-INF/"]}
    }])));

    assert!(files(&result, PlannedFileKind::Library).is_empty());
    let native = files(&result, PlannedFileKind::NativeLibrary)[0];
    assert_eq!(native.size, Some(64));
    assert_eq!(result.extractions.len(), 1);
    assert_eq!(result.extractions[0].source, native.destination);
    assert_eq!(
        result.extractions[0].destination,
        Path::new(".minecraft").join("versions/example/natives-linux-x86_64")
    );
    assert_eq!(result.extractions[0].exclude_prefixes, ["META-INF/"]);
}

#[test]
fn skips_native_entry_without_matching_os_key() {
    let result = plan(base_version(json!([{
        "name": "not a valid Maven coordinate and must not be read",
        "natives": {"windows": "natives-windows"}
    }])));

    assert_eq!(result.files.len(), 2);
    assert!(result.extractions.is_empty());
}

#[test]
fn rejects_missing_native_classifier_metadata() {
    let error = plan_vanilla(
        &layout(),
        &version(base_version(json!([{
            "name": "com.example:native:1.0",
            "natives": {"linux": "natives-linux"},
            "downloads": {"classifiers": {}}
        }]))),
        &linux(),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        InstallPlanError::MissingNativeClassifier { .. }
    ));
}

#[test]
fn supports_native_maven_fallback_when_downloads_are_absent() {
    let result = plan(base_version(json!([{
        "name": "com.example:native:1.0",
        "url": "https://example.invalid/maven/",
        "natives": {"linux": "natives-linux-${arch}"}
    }])));

    let native = files(&result, PlannedFileKind::NativeLibrary)[0];
    assert_eq!(
        native.sources[0].as_str(),
        "https://example.invalid/maven/com/example/native/1.0/native-1.0-natives-linux-64.jar"
    );
}

#[test]
fn rejects_unknown_architecture_for_native_template() {
    let platform = Platform::new(OperatingSystem::Linux, "6.1", "riscv64");
    let error = plan_vanilla(
        &layout(),
        &version(base_version(json!([{
            "name": "com.example:native:1.0",
            "natives": {"linux": "natives-linux-${arch}"}
        }]))),
        &platform,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        InstallPlanError::UnknownNativeArchitecture { .. }
    ));
}

#[test]
fn rejects_unresolved_native_placeholder() {
    let error = plan_vanilla(
        &layout(),
        &version(base_version(json!([{
            "name": "com.example:native:1.0",
            "natives": {"linux": "natives-${unknown}"}
        }]))),
        &linux(),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        InstallPlanError::UnresolvedNativeClassifier { .. }
    ));
}

#[test]
fn rejects_unresolved_inheritance_before_planning() {
    let mut inherited = base_version(json!([]));
    inherited["inheritsFrom"] = json!("parent");

    assert!(matches!(
        plan_vanilla(&layout(), &version(inherited), &linux()),
        Err(InstallPlanError::UnsupportedInheritance { .. })
    ));
}

#[test]
fn plans_loader_style_library_integrity_from_top_level_fields() {
    let plan = plan(base_version(json!([{
        "name": "net.fabricmc:fabric-loader:0.16.10",
        "url": "https://maven.fabricmc.net/",
        "sha1": HASH_B,
        "size": 42
    }])));

    let library = files(&plan, PlannedFileKind::Library)[0];
    assert_eq!(
        library.sha1.map(|hash| hash.to_string()).as_deref(),
        Some(HASH_B)
    );
    assert_eq!(library.size, Some(42));
}

#[test]
fn plans_client_redirect_and_rejects_missing_required_downloads() {
    let mut redirect = base_version(json!([]));
    redirect["jar"] = json!("base");
    let plan = plan_vanilla(&layout(), &version(redirect), &linux()).unwrap();
    let client = files(&plan, PlannedFileKind::Client)[0];
    assert_eq!(
        client.destination,
        Path::new(".minecraft").join("versions/base/base.jar")
    );

    let mut no_client = base_version(json!([]));
    no_client["downloads"] = json!({});
    assert!(matches!(
        plan_vanilla(&layout(), &version(no_client), &linux()),
        Err(InstallPlanError::MissingClientDownload(_))
    ));

    let mut no_index = base_version(json!([]));
    no_index.as_object_mut().unwrap().remove("assetIndex");
    assert!(matches!(
        plan_vanilla(&layout(), &version(no_index), &linux()),
        Err(InstallPlanError::MissingAssetIndex(_))
    ));
}

#[test]
fn rejects_unsafe_paths_and_non_http_urls() {
    let unsafe_path = base_version(json!([{
        "name": "com.example:core:1.0",
        "downloads": {
            "artifact": {
                "path": "../../evil.jar",
                "url": "https://example.invalid/evil.jar"
            }
        }
    }]));
    assert!(plan_vanilla(&layout(), &version(unsafe_path), &linux()).is_err());

    let unsafe_url = base_version(json!([{
        "name": "com.example:core:1.0",
        "downloads": {
            "artifact": {"path": "safe/core.jar", "url": "file:///tmp/core.jar"}
        }
    }]));
    assert!(matches!(
        plan_vanilla(&layout(), &version(unsafe_url), &linux()),
        Err(InstallPlanError::UnsupportedUrlScheme { .. })
    ));
}

#[test]
fn percent_encodes_repository_paths_in_download_urls() {
    let result = plan(base_version(json!([{
        "name": "com.example:core:1.0",
        "url": "https://example.invalid/maven",
        "downloads": {
            "artifact": {"path": "custom/core#demo.jar"}
        }
    }])));

    let library = files(&result, PlannedFileKind::Library)[0];
    assert_eq!(
        library.sources[0].as_str(),
        "https://example.invalid/maven/custom/core%23demo.jar"
    );
}

#[test]
fn rejects_missing_ordinary_artifact_metadata() {
    let error = plan_vanilla(
        &layout(),
        &version(base_version(json!([{
            "name": "com.example:core:1.0",
            "downloads": {}
        }]))),
        &linux(),
    )
    .unwrap_err();

    assert!(matches!(error, InstallPlanError::MissingLibraryArtifact(_)));
}

#[test]
fn deduplicates_identical_libraries() {
    let library = json!({
        "name": "com.example:core:1.0",
        "downloads": {
            "artifact": {
                "path": "custom/core.jar",
                "url": "https://example.invalid/core.jar",
                "sha1": HASH_A,
                "size": 100
            }
        }
    });
    let result = plan(base_version(json!([library.clone(), library])));

    assert_eq!(files(&result, PlannedFileKind::Library).len(), 1);
}

#[test]
fn rejects_conflicting_hashes_for_one_destination() {
    let make_library = |hash: &str| {
        json!({
            "name": "com.example:core:1.0",
            "downloads": {
                "artifact": {
                    "path": "custom/core.jar",
                    "url": "https://example.invalid/core.jar",
                    "sha1": hash,
                    "size": 100
                }
            }
        })
    };
    let error = plan_vanilla(
        &layout(),
        &version(base_version(json!([
            make_library(HASH_A),
            make_library(HASH_B)
        ]))),
        &linux(),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        InstallPlanError::ConflictingDownload { .. }
    ));
}

#[test]
fn rejects_unverified_alternate_sources_for_one_destination() {
    let make_library = |url: &str| {
        json!({
            "name": "com.example:core:1.0",
            "downloads": {
                "artifact": {"path": "custom/core.jar", "url": url, "size": 100}
            }
        })
    };
    let error = plan_vanilla(
        &layout(),
        &version(base_version(json!([
            make_library("https://one.invalid/core.jar"),
            make_library("https://two.invalid/core.jar")
        ]))),
        &linux(),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        InstallPlanError::ConflictingDownload { .. }
    ));
}

#[test]
fn plans_and_deduplicates_asset_objects_by_hash() {
    let catalog = AssetCatalog::from_json(
        &json!({
            "objects": {
                "a/first.txt": {"hash": HASH_A, "size": 10},
                "a/duplicate.txt": {"hash": HASH_A, "size": 10},
                "b/other.txt": {"hash": HASH_B, "size": 20}
            }
        })
        .to_string(),
    )
    .unwrap();

    let result = plan_asset_objects(&layout(), &catalog).unwrap();
    assert_eq!(result.files.len(), 2);
    let first = &result.files[0];
    assert_eq!(first.kind, PlannedFileKind::AssetObject);
    assert_eq!(first.size, Some(10));
    assert_eq!(
        first.destination,
        Path::new(".minecraft").join(format!("assets/objects/01/{HASH_A}"))
    );
    assert_eq!(
        first.sources[0].as_str(),
        format!("https://resources.download.minecraft.net/01/{HASH_A}")
    );
}

#[test]
fn plans_asset_objects_with_an_explicit_source_root() {
    let catalog = AssetCatalog::from_json(
        &json!({"objects": {"a.txt": {"hash": HASH_A, "size": 10}}}).to_string(),
    )
    .unwrap();
    let base = url::Url::parse("http://127.0.0.1:8080/assets/objects/").unwrap();

    let result = plan_asset_objects_with_base(&layout(), &catalog, &base).unwrap();

    assert_eq!(
        result.files[0].sources[0].as_str(),
        format!("http://127.0.0.1:8080/assets/objects/01/{HASH_A}")
    );
    assert_eq!(result.files[0].sha1, Some(HASH_A.parse().unwrap()));
}

#[test]
fn rejects_unsupported_asset_layouts_and_unsafe_source_roots() {
    let virtual_catalog =
        AssetCatalog::from_json(&json!({"virtual": true, "objects": {}}).to_string()).unwrap();
    assert!(matches!(
        plan_asset_objects(&layout(), &virtual_catalog),
        Err(InstallPlanError::UnsupportedVirtualAssets)
    ));

    let mapped_catalog =
        AssetCatalog::from_json(&json!({"map_to_resources": true, "objects": {}}).to_string())
            .unwrap();
    assert!(matches!(
        plan_asset_objects(&layout(), &mapped_catalog),
        Err(InstallPlanError::UnsupportedMappedAssets)
    ));

    let catalog = AssetCatalog::default();
    let signed = url::Url::parse("https://example.invalid/assets/?token=secret").unwrap();
    let error = plan_asset_objects_with_base(&layout(), &catalog, &signed).unwrap_err();
    assert!(matches!(error, InstallPlanError::InvalidAssetObjectBase));
    assert!(!error.to_string().contains("secret"));
}

#[test]
fn rejects_asset_size_conflicts_and_invalid_hashes() {
    let conflicting = AssetCatalog::from_json(
        &json!({
            "objects": {
                "a.txt": {"hash": HASH_A, "size": 10},
                "b.txt": {"hash": HASH_A, "size": 11}
            }
        })
        .to_string(),
    )
    .unwrap();
    assert!(matches!(
        plan_asset_objects(&layout(), &conflicting),
        Err(InstallPlanError::ConflictingDownload { .. })
    ));

    let invalid = AssetCatalog::from_json(
        &json!({"objects": {"bad.txt": {"hash": "../bad", "size": 1}}}).to_string(),
    )
    .unwrap();
    assert!(plan_asset_objects(&layout(), &invalid).is_err());
}

#[test]
fn rejects_unsafe_asset_index_id() {
    for id in ["../escape", " "] {
        let mut metadata = base_version(json!([]));
        metadata["assetIndex"]["id"] = json!(id);
        assert!(plan_vanilla(&layout(), &version(metadata), &linux()).is_err());
    }
}
