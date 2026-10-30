use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use flate2::{Compression, write::GzEncoder};
use rmcl_world::{
    CopyStrategy, CopyWorldOptions, ImportWorldOptions, WorldCompatibility, WorldConflictPolicy,
    backup_world, client_world_version, copy_world, delete_world, export_world_zip,
    import_world_zip, move_world, read_world, restore_world_backup, scan_worlds,
};
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
    last_played: i64,
    version: Version,
    was_modded: i8,
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
    fs::create_dir_all(world.join("data")).unwrap();
    let root = Root {
        data: Data {
            data_version: 3955,
            level_name: format!("§a{name}"),
            last_played: 1_700_000_000_000,
            version: Version {
                id: 3955,
                name: "1.21.1".to_owned(),
            },
            was_modded: 0,
        },
    };
    let bytes = fastnbt::to_bytes(&root).unwrap();
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&bytes).unwrap();
    fs::write(world.join("level.dat"), encoder.finish().unwrap()).unwrap();
    fs::write(world.join("region").join("r.0.0.mca"), b"region").unwrap();
    fs::write(world.join("data").join("chunks.dat"), b"data").unwrap();
    fs::write(world.join("session.lock"), b"lock").unwrap();
    world
}

#[test]
fn scans_metadata_and_hides_formatting_codes() {
    let temporary = TempDir::new().unwrap();
    let saves = temporary.path().join("saves");
    write_world(&saves, "Survival");

    let scan = scan_worlds(&saves).unwrap();
    assert_eq!(scan.worlds.len(), 1);
    assert!(scan.warnings.is_empty());
    let world = &scan.worlds[0];
    assert_eq!(world.name, "Survival");
    assert_eq!(world.game_version.as_deref(), Some("1.21.1"));
    assert_eq!(world.data_version, Some(3955));
    assert_eq!(world.compatibility(Some(3955)), WorldCompatibility::Same);
    assert_eq!(world.compatibility(Some(4000)), WorldCompatibility::Upgrade);
    assert_eq!(
        world.compatibility(Some(3900)),
        WorldCompatibility::Downgrade
    );
}

#[test]
fn copy_renames_on_conflict_and_creates_an_independent_world() {
    let temporary = TempDir::new().unwrap();
    let source_saves = temporary.path().join("source");
    let target_saves = temporary.path().join("target");
    write_world(&source_saves, "Survival");
    write_world(&target_saves, "Survival");
    let source = read_world(source_saves.join("Survival")).unwrap();

    let outcome = copy_world(
        &source,
        &target_saves,
        &CopyWorldOptions::default(),
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(outcome.world.folder_name(), "Survival (1)");
    assert!(!outcome.target_path.join("session.lock").exists());
    fs::write(outcome.target_path.join("level.dat"), b"changed").unwrap();
    assert_ne!(
        fs::read(source.path.join("level.dat")).unwrap(),
        b"changed".to_vec()
    );
}

#[test]
fn cancelled_copy_leaves_no_partial_world() {
    let temporary = TempDir::new().unwrap();
    let source_saves = temporary.path().join("source");
    let target_saves = temporary.path().join("target");
    write_world(&source_saves, "Survival");
    let source = read_world(source_saves.join("Survival")).unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let error = copy_world(
        &source,
        &target_saves,
        &CopyWorldOptions::default(),
        &cancellation,
        |_| {},
    )
    .unwrap_err();
    assert!(matches!(error, rmcl_world::WorldError::Cancelled));
    assert!(!target_saves.join("Survival").exists());
}

#[test]
fn replace_publishes_the_new_world() {
    let temporary = TempDir::new().unwrap();
    let source_saves = temporary.path().join("source");
    let target_saves = temporary.path().join("target");
    let source_path = write_world(&source_saves, "Survival");
    write_world(&target_saves, "Survival");
    fs::write(
        target_saves.join("Survival/region/r.0.0.mca"),
        b"old-target",
    )
    .unwrap();
    let source = read_world(&source_path).unwrap();

    copy_world(
        &source,
        &target_saves,
        &CopyWorldOptions {
            conflict: WorldConflictPolicy::Replace,
            strategy: CopyStrategy::Copy,
            ..CopyWorldOptions::default()
        },
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();

    assert_eq!(
        fs::read(target_saves.join("Survival/region/r.0.0.mca")).unwrap(),
        b"region"
    );
}

#[cfg(unix)]
#[test]
fn scan_skips_symlinked_world_directories() {
    let temporary = TempDir::new().unwrap();
    let saves = temporary.path().join("saves");
    let world = write_world(&saves, "Survival");
    std::os::unix::fs::symlink(&world, saves.join("Linked")).unwrap();

    let scan = scan_worlds(&saves).unwrap();
    assert_eq!(scan.worlds.len(), 1);
    assert_eq!(scan.warnings.len(), 1);
}

#[test]
fn copy_rejects_symbolic_links_inside_a_world() {
    #[cfg(unix)]
    {
        let temporary = TempDir::new().unwrap();
        let source_saves = temporary.path().join("source");
        let target_saves = temporary.path().join("target");
        let world = write_world(&source_saves, "Survival");
        std::os::unix::fs::symlink("/tmp", world.join("escape")).unwrap();
        let source = read_world(&world).unwrap();

        let error = copy_world(
            &source,
            &target_saves,
            &CopyWorldOptions {
                strategy: CopyStrategy::Copy,
                ..CopyWorldOptions::default()
            },
            &CancellationToken::new(),
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(
            error,
            rmcl_world::WorldError::SymlinkNotAllowed { .. }
        ));
        assert!(!target_saves.join("Survival").exists());
    }
}

#[test]
fn export_skips_session_lock_and_import_round_trips() {
    let temporary = TempDir::new().unwrap();
    let source_saves = temporary.path().join("source");
    let target_saves = temporary.path().join("target");
    let world_path = write_world(&source_saves, "Survival");
    let world = read_world(&world_path).unwrap();
    let archive = temporary.path().join("Survival.zip");

    export_world_zip(&world, &archive, &CancellationToken::new(), |_| {}).unwrap();
    let file = fs::File::open(&archive).unwrap();
    let mut zip = zip::ZipArchive::new(file).unwrap();
    assert!(zip.by_name("Survival/level.dat").is_ok());
    assert!(zip.by_name("Survival/session.lock").is_err());

    let imported = import_world_zip(
        &archive,
        &target_saves,
        &ImportWorldOptions::default(),
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(imported.name, "Survival");
    assert!(!imported.path.join("session.lock").exists());
    assert_eq!(
        fs::read(imported.path.join("region/r.0.0.mca")).unwrap(),
        b"region"
    );
}

#[test]
fn backup_and_restore_are_plain_zip_operations() {
    let temporary = TempDir::new().unwrap();
    let saves = temporary.path().join("saves");
    let backups = temporary.path().join("backups");
    let world_path = write_world(&saves, "Survival");
    let world = read_world(&world_path).unwrap();

    let backup = backup_world(&world, &backups, &CancellationToken::new(), |_| {}).unwrap();
    assert!(backup.path.is_file());

    let restored = restore_world_backup(
        &backup.path,
        &temporary.path().join("restored"),
        &ImportWorldOptions::default(),
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(restored.name, "Survival");
}

#[test]
fn move_renames_same_filesystem_worlds() {
    let temporary = TempDir::new().unwrap();
    let source_saves = temporary.path().join("source");
    let target_saves = temporary.path().join("target");
    let world_path = write_world(&source_saves, "Survival");
    let world = read_world(&world_path).unwrap();

    let outcome = move_world(
        &world,
        &target_saves,
        &CopyWorldOptions::default(),
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(outcome.world.name, "Survival");
    assert!(!world_path.exists());
    assert!(target_saves.join("Survival").is_dir());
}

#[test]
fn delete_removes_a_world() {
    let temporary = TempDir::new().unwrap();
    let saves = temporary.path().join("saves");
    let world_path = write_world(&saves, "Survival");
    let world = read_world(&world_path).unwrap();
    delete_world(&world).unwrap();
    assert!(!world_path.exists());
}

#[test]
fn reads_world_version_from_client_jar() {
    let temporary = TempDir::new().unwrap();
    let jar = temporary.path().join("client.jar");
    let file = fs::File::create(&jar).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file("version.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(br#"{"id":"1.21.1","world_version":3955}"#)
        .unwrap();
    zip.finish().unwrap();

    assert_eq!(client_world_version(&jar).unwrap(), Some(3955));
    assert_eq!(
        client_world_version(&temporary.path().join("missing.jar")).unwrap(),
        None
    );
}

#[test]
fn reject_conflict_does_not_write_a_world() {
    let temporary = TempDir::new().unwrap();
    let source_saves = temporary.path().join("source");
    let target_saves = temporary.path().join("target");
    write_world(&source_saves, "Survival");
    write_world(&target_saves, "Survival");
    let source = read_world(source_saves.join("Survival")).unwrap();

    let error = copy_world(
        &source,
        &target_saves,
        &CopyWorldOptions {
            conflict: WorldConflictPolicy::Reject,
            ..CopyWorldOptions::default()
        },
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap_err();
    assert!(matches!(error, rmcl_world::WorldError::WorldExists(_)));
    assert_eq!(
        fs::read(target_saves.join("Survival/region/r.0.0.mca")).unwrap(),
        b"region"
    );
}
