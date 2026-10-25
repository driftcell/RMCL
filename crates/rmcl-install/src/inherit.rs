use std::collections::HashSet;

use rmcl_instance::{MinecraftLayout, VersionId};
use rmcl_minecraft::{Arguments, GameVersion};

use crate::InstallPlanError;

const MAX_INHERITANCE_DEPTH: usize = 16;

/// Resolves a version JSON and all of its `inheritsFrom` parents into one
/// standalone profile.
///
/// Parent libraries and arguments are placed before child entries; child
/// scalar fields and downloads override the parent. The returned version has
/// `inheritsFrom = None` and a resolved `jar` pointing at the base version when
/// the child does not provide one.
pub fn resolve_version(
    layout: &MinecraftLayout,
    id: &VersionId,
) -> Result<GameVersion, InstallPlanError> {
    let mut chain: Vec<GameVersion> = Vec::new();
    let mut seen = HashSet::new();
    let mut current = id.clone();

    loop {
        if chain.len() >= MAX_INHERITANCE_DEPTH {
            return Err(InstallPlanError::InheritanceDepth {
                id: id.as_str().to_owned(),
                max_depth: MAX_INHERITANCE_DEPTH,
            });
        }
        if !seen.insert(current.as_str().to_owned()) {
            return Err(InstallPlanError::InheritanceCycle {
                id: id.as_str().to_owned(),
            });
        }

        let path = layout.version_json_path(&current);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Err(match chain.last() {
                    Some(child) => InstallPlanError::MissingParent {
                        id: child.id.clone(),
                        parent: current.as_str().to_owned(),
                    },
                    None => InstallPlanError::VersionNotFound(path.display().to_string()),
                });
            }
            Err(source) => {
                return Err(InstallPlanError::VersionRead { path, source });
            }
        };
        let version = GameVersion::from_json(&text)?;
        if version.id != current.as_str() {
            return Err(InstallPlanError::VersionIdMismatch {
                path: path.display().to_string(),
                expected: current.as_str().to_owned(),
                actual: version.id.clone(),
            });
        }
        let parent = version.inherits_from.clone();
        chain.push(version);
        match parent {
            Some(parent) => current = VersionId::new(parent)?,
            None => break,
        }
    }

    chain.reverse();
    let mut iterations = chain.into_iter();
    let mut merged = iterations.next().ok_or_else(|| {
        InstallPlanError::VersionNotFound(layout.version_json_path(id).display().to_string())
    })?;
    for child in iterations {
        merged = merge_versions(merged, child);
    }
    merged.inherits_from = None;
    Ok(merged)
}

fn merge_versions(parent: GameVersion, child: GameVersion) -> GameVersion {
    let GameVersion {
        id: parent_id,
        jar: parent_jar,
        main_class: parent_main_class,
        arguments: parent_arguments,
        minecraft_arguments: parent_legacy_arguments,
        libraries: parent_libraries,
        downloads: parent_downloads,
        asset_index: parent_asset_index,
        assets: parent_assets,
        java_version: parent_java_version,
        minimum_launcher_version: parent_minimum_launcher_version,
        compliance_level: parent_compliance_level,
        time: parent_time,
        release_time: parent_release_time,
        version_type: parent_version_type,
        ..
    } = parent;
    let GameVersion {
        id,
        inherits_from,
        jar,
        main_class,
        arguments,
        minecraft_arguments,
        libraries,
        downloads,
        asset_index,
        assets,
        java_version,
        minimum_launcher_version,
        compliance_level,
        time,
        release_time,
        version_type,
    } = child;

    let mut merged_downloads = parent_downloads;
    merged_downloads.extend(downloads);

    GameVersion {
        id,
        inherits_from,
        jar: jar.or(parent_jar).or(Some(parent_id)),
        main_class: main_class.or(parent_main_class),
        arguments: merge_arguments(parent_arguments, arguments),
        minecraft_arguments: minecraft_arguments.or(parent_legacy_arguments),
        libraries: merge_libraries(parent_libraries, libraries),
        downloads: merged_downloads,
        asset_index: asset_index.or(parent_asset_index),
        assets: assets.or(parent_assets),
        java_version: java_version.or(parent_java_version),
        minimum_launcher_version: parent_minimum_launcher_version.max(minimum_launcher_version),
        compliance_level: parent_compliance_level.max(compliance_level),
        time: time.or(parent_time),
        release_time: release_time.or(parent_release_time),
        version_type: version_type.or(parent_version_type),
    }
}

fn merge_libraries(
    parent: Vec<rmcl_minecraft::Library>,
    child: Vec<rmcl_minecraft::Library>,
) -> Vec<rmcl_minecraft::Library> {
    let child_keys = child
        .iter()
        .filter_map(|library| library_key(&library.name))
        .collect::<HashSet<_>>();
    let mut merged = parent
        .into_iter()
        .filter(|library| library_key(&library.name).is_none_or(|key| !child_keys.contains(&key)))
        .collect::<Vec<_>>();
    merged.extend(child);
    merged
}

/// Groups libraries across versions while preserving classifier/extension.
fn library_key(name: &str) -> Option<String> {
    let (name, extension) = name
        .split_once('@')
        .map_or((name, "jar"), |(name, extension)| (name, extension));
    let parts = name.split(':').collect::<Vec<_>>();
    if parts.len() < 3 || parts[0].is_empty() || parts[1].is_empty() {
        return None;
    }
    let classifier = parts.get(3).copied().unwrap_or_default();
    Some(format!(
        "{}:{}:{}:@{}",
        parts[0], parts[1], classifier, extension
    ))
}

fn merge_arguments(parent: Option<Arguments>, child: Option<Arguments>) -> Option<Arguments> {
    match (parent, child) {
        (None, None) => None,
        (Some(parent), None) => Some(parent),
        (None, Some(child)) => Some(child),
        (Some(mut parent), Some(child)) => {
            parent.game.extend(child.game);
            parent.jvm.extend(child.jvm);
            Some(parent)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rmcl_instance::MinecraftLayout;
    use rmcl_minecraft::GameVersion;

    use super::*;

    fn write_version(root: &Path, id: &str, value: serde_json::Value) {
        let directory = root.join("versions").join(id);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(format!("{id}.json")), value.to_string()).unwrap();
    }

    #[test]
    fn merges_parent_and_child_fields() {
        let temporary = tempfile::TempDir::new().unwrap();
        write_version(
            temporary.path(),
            "base",
            serde_json::json!({
                "id": "base",
                "mainClass": "base.Main",
                "type": "release",
                "releaseTime": "2025-01-01T00:00:00Z",
                "arguments": {
                    "jvm": ["-Dparent=true"],
                    "game": ["--parent"]
                },
                "downloads": {
                    "client": {
                        "url": "https://example.invalid/client.jar",
                        "sha1": "0123456789abcdef0123456789abcdef01234567",
                        "size": 10
                    }
                },
                "assetIndex": {
                    "id": "index",
                    "url": "https://example.invalid/index.json"
                },
                "libraries": [{
                    "name": "example:base:1.0",
                    "url": "https://example.invalid/"
                }]
            }),
        );
        write_version(
            temporary.path(),
            "child",
            serde_json::json!({
                "id": "child",
                "inheritsFrom": "base",
                "mainClass": "child.Main",
                "arguments": {
                    "jvm": ["-Dchild=true"],
                    "game": ["--child"]
                },
                "libraries": [{
                    "name": "example:child:1.0",
                    "url": "https://example.invalid/"
                }]
            }),
        );
        let layout = MinecraftLayout::new(temporary.path());
        let id = VersionId::new("child").unwrap();

        let resolved = resolve_version(&layout, &id).unwrap();

        assert_eq!(resolved.id, "child");
        assert_eq!(resolved.inherits_from, None);
        assert_eq!(resolved.jar.as_deref(), Some("base"));
        assert_eq!(resolved.main_class.as_deref(), Some("child.Main"));
        assert_eq!(resolved.libraries.len(), 2);
        assert_eq!(resolved.libraries[0].name, "example:base:1.0");
        assert_eq!(resolved.libraries[1].name, "example:child:1.0");
        assert!(resolved.downloads.contains_key("client"));
        assert_eq!(
            resolved.asset_index.as_ref().map(|index| index.id.as_str()),
            Some("index")
        );
        let arguments = resolved.arguments.unwrap();
        assert_eq!(arguments.jvm.len(), 2);
        assert_eq!(arguments.game.len(), 2);
        assert_eq!(resolved.version_type.as_deref(), Some("release"));
    }

    #[test]
    fn child_libraries_override_matching_parent_coordinates() {
        let temporary = tempfile::TempDir::new().unwrap();
        write_version(
            temporary.path(),
            "base",
            serde_json::json!({
                "id": "base",
                "libraries": [
                    {"name": "org.ow2.asm:asm:9.5", "url": "https://example.invalid/"},
                    {"name": "org.lwjgl:lwjgl:3.3.2:natives-linux", "url": "https://example.invalid/"},
                    {"name": "org.lwjgl:lwjgl:3.3.2", "url": "https://example.invalid/"}
                ]
            }),
        );
        write_version(
            temporary.path(),
            "child",
            serde_json::json!({
                "id": "child",
                "inheritsFrom": "base",
                "libraries": [
                    {"name": "org.ow2.asm:asm:9.10.1", "url": "https://example.invalid/"},
                    {"name": "org.lwjgl:lwjgl:3.3.3:natives-linux", "url": "https://example.invalid/"}
                ]
            }),
        );
        let layout = MinecraftLayout::new(temporary.path());
        let resolved = resolve_version(&layout, &VersionId::new("child").unwrap()).unwrap();
        let names = resolved
            .libraries
            .iter()
            .map(|library| library.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            [
                "org.lwjgl:lwjgl:3.3.2",
                "org.ow2.asm:asm:9.10.1",
                "org.lwjgl:lwjgl:3.3.3:natives-linux"
            ]
        );
    }

    #[test]
    fn rejects_cycles_and_missing_parents() {
        let temporary = tempfile::TempDir::new().unwrap();
        write_version(
            temporary.path(),
            "a",
            serde_json::json!({"id": "a", "inheritsFrom": "b"}),
        );
        write_version(
            temporary.path(),
            "b",
            serde_json::json!({"id": "b", "inheritsFrom": "a"}),
        );
        write_version(
            temporary.path(),
            "orphan",
            serde_json::json!({"id": "orphan", "inheritsFrom": "missing"}),
        );
        let layout = MinecraftLayout::new(temporary.path());

        assert!(matches!(
            resolve_version(&layout, &VersionId::new("a").unwrap()),
            Err(InstallPlanError::InheritanceCycle { .. })
        ));
        assert!(matches!(
            resolve_version(&layout, &VersionId::new("orphan").unwrap()),
            Err(InstallPlanError::MissingParent { .. })
        ));
        assert!(matches!(
            resolve_version(&layout, &VersionId::new("absent").unwrap()),
            Err(InstallPlanError::VersionNotFound { .. })
        ));
    }

    #[test]
    fn merged_profile_can_be_parsed_again() {
        let temporary = tempfile::TempDir::new().unwrap();
        write_version(
            temporary.path(),
            "base",
            serde_json::json!({"id": "base", "mainClass": "base.Main"}),
        );
        write_version(
            temporary.path(),
            "child",
            serde_json::json!({"id": "child", "inheritsFrom": "base"}),
        );
        let layout = MinecraftLayout::new(temporary.path());
        let resolved = resolve_version(&layout, &VersionId::new("child").unwrap()).unwrap();

        assert_eq!(
            resolved,
            GameVersion::from_json(
                &serde_json::json!({
                    "id": "child",
                    "jar": "base",
                    "mainClass": "base.Main",
                    "libraries": [],
                    "downloads": {}
                })
                .to_string()
            )
            .unwrap()
        );
    }
}
