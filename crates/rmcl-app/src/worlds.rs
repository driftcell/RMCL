use std::path::{Path, PathBuf};

use rmcl_instance::InstanceLayout;
use rmcl_world::{
    CopyWorldOptions, CopyWorldOutcome, ImportWorldOptions, World, WorldBackup,
    WorldConflictPolicy, WorldCopyProgress, WorldId, WorldScan, WorldWarning, backup_world,
    client_world_version, copy_world, delete_world, export_world_zip, import_world_zip, move_world,
    read_world, restore_world_backup, scan_worlds,
};
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use crate::{AppError, RepositoryWriteGuard, try_lock_repository};

/// One instance's worlds plus the target client data version.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstanceWorlds {
    pub worlds: Vec<World>,
    pub warnings: Vec<WorldWarning>,
    pub target_world_version: Option<i32>,
    pub client_jar_error: Option<String>,
}

/// Scans an instance world directory and reads its client data version.
pub fn inspect_instance_worlds(
    instance: &InstanceLayout,
    client_jar: &Path,
) -> Result<InstanceWorlds, AppError> {
    let WorldScan { worlds, warnings } = scan_worlds(instance.saves_dir())?;
    let (target_world_version, client_jar_error) = match client_world_version(client_jar) {
        Ok(version) => (version, None),
        Err(error) => (None, Some(error.to_string())),
    };
    Ok(InstanceWorlds {
        worlds,
        warnings,
        target_world_version,
        client_jar_error,
    })
}

/// Copies one world between instances in the same repository.
pub async fn copy_world_to_instance(
    source: InstanceLayout,
    target: InstanceLayout,
    world_id: WorldId,
    target_world_version: Option<i32>,
    options: CopyWorldOptions,
    cancellation: CancellationToken,
    progress: Option<UnboundedSender<WorldCopyProgress>>,
) -> Result<CopyWorldOutcome, AppError> {
    let root = shared_root(&source, &target)?;
    tokio::task::spawn_blocking(move || {
        let _guard: RepositoryWriteGuard = try_lock_repository(&root)?;
        let source_world = read_world(source.saves_dir().join(world_id.as_str()))?;
        backup_before_upgrade(&source_world, &target, target_world_version, &cancellation)?;
        if options.conflict == WorldConflictPolicy::Replace {
            backup_existing_target(&target, &world_id, &options, &cancellation)?;
        }
        copy_world(
            &source_world,
            &target.saves_dir(),
            &options,
            &cancellation,
            |event| send_progress(&progress, event),
        )
        .map_err(AppError::from)
    })
    .await
    .map_err(|error| AppError::BlockingTask(error.to_string()))?
}

/// Moves one world between instances in the same repository.
pub async fn move_world_to_instance(
    source: InstanceLayout,
    target: InstanceLayout,
    world_id: WorldId,
    target_world_version: Option<i32>,
    options: CopyWorldOptions,
    cancellation: CancellationToken,
    progress: Option<UnboundedSender<WorldCopyProgress>>,
) -> Result<CopyWorldOutcome, AppError> {
    let root = shared_root(&source, &target)?;
    tokio::task::spawn_blocking(move || {
        let _guard: RepositoryWriteGuard = try_lock_repository(&root)?;
        let source_world = read_world(source.saves_dir().join(world_id.as_str()))?;
        backup_before_upgrade(&source_world, &target, target_world_version, &cancellation)?;
        if options.conflict == WorldConflictPolicy::Replace {
            backup_existing_target(&target, &world_id, &options, &cancellation)?;
        }
        move_world(
            &source_world,
            &target.saves_dir(),
            &options,
            &cancellation,
            |event| send_progress(&progress, event),
        )
        .map_err(AppError::from)
    })
    .await
    .map_err(|error| AppError::BlockingTask(error.to_string()))?
}

/// Imports a world ZIP into an instance.
pub async fn import_instance_world(
    instance: InstanceLayout,
    archive: PathBuf,
    options: ImportWorldOptions,
    cancellation: CancellationToken,
    progress: Option<UnboundedSender<WorldCopyProgress>>,
) -> Result<World, AppError> {
    let root = instance.repository().root().to_path_buf();
    tokio::task::spawn_blocking(move || {
        let _guard = try_lock_repository(&root)?;
        import_world_zip(
            &archive,
            &instance.saves_dir(),
            &options,
            &cancellation,
            |event| send_progress(&progress, event),
        )
        .map_err(AppError::from)
    })
    .await
    .map_err(|error| AppError::BlockingTask(error.to_string()))?
}

/// Exports a world ZIP to a user-selected destination.
pub async fn export_instance_world(
    instance: InstanceLayout,
    world_id: WorldId,
    destination: PathBuf,
    cancellation: CancellationToken,
    progress: Option<UnboundedSender<WorldCopyProgress>>,
) -> Result<PathBuf, AppError> {
    let root = instance.repository().root().to_path_buf();
    tokio::task::spawn_blocking(move || {
        let _guard = try_lock_repository(&root)?;
        let world = read_world(instance.saves_dir().join(world_id.as_str()))?;
        export_world_zip(&world, &destination, &cancellation, |event| {
            send_progress(&progress, event)
        })?;
        Ok::<_, AppError>(destination)
    })
    .await
    .map_err(|error| AppError::BlockingTask(error.to_string()))?
}

/// Backs up one world below the instance's `backups/` directory.
pub async fn backup_instance_world(
    instance: InstanceLayout,
    world_id: WorldId,
    cancellation: CancellationToken,
    progress: Option<UnboundedSender<WorldCopyProgress>>,
) -> Result<WorldBackup, AppError> {
    let root = instance.repository().root().to_path_buf();
    tokio::task::spawn_blocking(move || {
        let _guard = try_lock_repository(&root)?;
        let world = read_world(instance.saves_dir().join(world_id.as_str()))?;
        backup_world(&world, &backups_dir(&instance), &cancellation, |event| {
            send_progress(&progress, event)
        })
        .map_err(AppError::from)
    })
    .await
    .map_err(|error| AppError::BlockingTask(error.to_string()))?
}

/// Restores a ZIP backup into an instance.
pub async fn restore_instance_world(
    instance: InstanceLayout,
    backup: PathBuf,
    options: ImportWorldOptions,
    cancellation: CancellationToken,
    progress: Option<UnboundedSender<WorldCopyProgress>>,
) -> Result<World, AppError> {
    let root = instance.repository().root().to_path_buf();
    tokio::task::spawn_blocking(move || {
        let _guard = try_lock_repository(&root)?;
        restore_world_backup(
            &backup,
            &instance.saves_dir(),
            &options,
            &cancellation,
            |event| send_progress(&progress, event),
        )
        .map_err(AppError::from)
    })
    .await
    .map_err(|error| AppError::BlockingTask(error.to_string()))?
}

/// Deletes one world after verifying that it is not in use.
pub async fn delete_instance_world(
    instance: InstanceLayout,
    world_id: WorldId,
) -> Result<(), AppError> {
    let root = instance.repository().root().to_path_buf();
    tokio::task::spawn_blocking(move || {
        let _guard = try_lock_repository(&root)?;
        let world = read_world(instance.saves_dir().join(world_id.as_str()))?;
        delete_world(&world).map_err(AppError::from)
    })
    .await
    .map_err(|error| AppError::BlockingTask(error.to_string()))?
}

pub fn backups_dir(instance: &InstanceLayout) -> PathBuf {
    instance.game_dir().join("backups")
}

fn shared_root(source: &InstanceLayout, target: &InstanceLayout) -> Result<PathBuf, AppError> {
    let source_root = source.repository().root();
    if source_root != target.repository().root() {
        return Err(AppError::InvalidRequest(
            "world transfer requires both instances to share one Minecraft repository".to_owned(),
        ));
    }
    Ok(source_root.to_path_buf())
}

fn backup_before_upgrade(
    source_world: &World,
    target: &InstanceLayout,
    target_world_version: Option<i32>,
    cancellation: &CancellationToken,
) -> Result<(), AppError> {
    let Some(target_version) = target_world_version else {
        return Ok(());
    };
    if source_world
        .data_version
        .is_some_and(|source_version| source_version < target_version)
    {
        backup_world(source_world, &backups_dir(target), cancellation, |_| {})?;
    }
    Ok(())
}

fn backup_existing_target(
    target: &InstanceLayout,
    source_id: &WorldId,
    options: &CopyWorldOptions,
    cancellation: &CancellationToken,
) -> Result<(), AppError> {
    let target_name = match options.target_name.as_deref() {
        Some(name) => WorldId::new(name)?,
        None => source_id.clone(),
    };
    let target_path = target.saves_dir().join(target_name.as_str());
    if !target_path.exists() {
        return Ok(());
    }
    let target_world = read_world(&target_path)?;
    backup_world(&target_world, &backups_dir(target), cancellation, |_| {})?;
    Ok(())
}

fn send_progress(sender: &Option<UnboundedSender<WorldCopyProgress>>, event: WorldCopyProgress) {
    if let Some(sender) = sender {
        let _ = sender.send(event);
    }
}
