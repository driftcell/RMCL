use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

use crate::WorldError;

/// An exclusive lock on a world's `session.lock`.
///
/// Minecraft uses `FileChannel.tryLock()`. On Unix that is a POSIX `fcntl`
/// record lock, not `flock`, so RMCL must use the same primitive to detect a
/// running game.
#[derive(Debug)]
pub struct WorldLock {
    file: File,
    path: PathBuf,
}

impl WorldLock {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for WorldLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let _ =
                rustix::fs::fcntl_lock(&self.file, rustix::fs::FlockOperation::NonBlockingUnlock);
        }
        #[cfg(windows)]
        {
            let _ = self.file.unlock();
        }
    }
}

/// Attempts to take an exclusive world lock without blocking.
///
/// Returns `Ok(None)` when another process (normally Minecraft) holds the
/// lock. The lock file is created when missing, matching the game's behavior.
pub fn try_lock_world(world_dir: &Path) -> Result<Option<WorldLock>, WorldError> {
    let path = world_dir.join("session.lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|source| WorldError::io(&path, source))?;

    #[cfg(unix)]
    {
        match rustix::fs::fcntl_lock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => Ok(Some(WorldLock { file, path })),
            Err(error)
                if error == rustix::io::Errno::AGAIN || error == rustix::io::Errno::ACCESS =>
            {
                Ok(None)
            }
            Err(error) => Err(WorldError::io(&path, error.into())),
        }
    }

    #[cfg(windows)]
    {
        match file.try_lock() {
            Ok(()) => Ok(Some(WorldLock { file, path })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(WorldError::io(&path, error)),
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Ok(Some(WorldLock { file, path }))
    }
}

/// Best-effort lock check used while scanning a list of worlds.
pub fn is_world_locked(world_dir: &Path) -> bool {
    if !world_dir.join("session.lock").is_file() {
        return false;
    }
    matches!(try_lock_world(world_dir), Ok(None))
}
