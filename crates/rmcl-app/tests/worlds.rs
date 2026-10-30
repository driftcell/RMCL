use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use flate2::{Compression, write::GzEncoder};
use rmcl_app::{
    CopyStrategy, CopyWorldOptions, WorldConflictPolicy, WorldId, copy_world_to_instance,
    inspect_instance_worlds,
};
use rmcl_instance::{InstanceLayout, IsolationMode, MinecraftLayout, VersionId};
use serde::Serialize;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct Root {
    data: Data,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct Data {
    data_version: i32,
    level_name: String,
    version: Version,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct Version {
    id: i32,
    name: String,
}

fn write_world(saves: &Path, name: &str) -> PathBuf {
    let world = saves.join(name);
    fs::create_dir_all(world.join("region")).unwrap();
    let root = Root {
        data: Data {
            data_version: 3955,
            level_name: name.to_owned(),
            version: Version {
                id: 3955,
                name: "1.21.1".to_owned(),
            },
        },
    };
    let bytes = fastnbt::to_bytes(&root).unwrap();
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&bytes).unwrap();
    fs::write(world.join("level.dat"), encoder.finish().unwrap()).unwrap();
    fs::write(world.join("region/r.0.0.mca"), b"region").unwrap();
    world
}

fn instance(root: &Path, id: &str) -> InstanceLayout {
    MinecraftLayout::new(root).instance(VersionId::new(id).unwrap(), IsolationMode::Isolated)
}

fn client_jar(root: &Path, id: &str, world_version: i32) -> PathBuf {
    let path = root.join("versions").join(id).join(format!("{id}.jar"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = fs::File::create(&path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file("version.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    write!(zip, r#"{{"world_version":{world_version}}}"#).unwrap();
    zip.finish().unwrap();
    path
}

#[test]
fn inspect_reports_the_target_data_version() {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path();
    let source = instance(root, "source");
    write_world(&source.saves_dir(), "Survival");
    let jar = client_jar(root, "source", 3955);

    let inspection = inspect_instance_worlds(&source, &jar).unwrap();
    assert_eq!(inspection.target_world_version, Some(3955));
    assert_eq!(inspection.worlds.len(), 1);
    assert_eq!(inspection.worlds[0].name, "Survival");
}

#[tokio::test]
async fn copy_between_instances_creates_an_upgrade_backup() {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path();
    let source = instance(root, "source");
    let target = instance(root, "target");
    write_world(&source.saves_dir(), "Survival");

    let outcome = copy_world_to_instance(
        source.clone(),
        target.clone(),
        WorldId::new("Survival").unwrap(),
        Some(4000),
        CopyWorldOptions {
            conflict: WorldConflictPolicy::Rename,
            strategy: CopyStrategy::Copy,
            ..CopyWorldOptions::default()
        },
        CancellationToken::new(),
        None,
    )
    .await
    .unwrap();

    assert!(source.saves_dir().join("Survival/level.dat").is_file());
    assert!(outcome.target_path.join("level.dat").is_file());
    let backups = target.game_dir().join("backups");
    assert!(
        fs::read_dir(backups).unwrap().next().is_some(),
        "upgrade copy should create a backup"
    );
}
