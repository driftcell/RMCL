use std::{
    fs::{File, OpenOptions, TryLockError},
    path::{Path, PathBuf},
};

use crate::AppError;

const REPOSITORY_LOCK_FILE: &str = ".rmcl.lock";

/// Exclusive advisory lock for one Minecraft repository.
///
/// The guard is held by installation, content and game tasks until they reach a
/// terminal state. Short synchronous writers hold it for their whole operation.
/// Closing the file releases the lock on all supported platforms; explicit
/// unlock in `Drop` keeps that lifecycle clear.
#[derive(Debug)]
pub struct RepositoryWriteGuard {
    file: File,
    path: PathBuf,
}

impl RepositoryWriteGuard {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RepositoryWriteGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Attempts to lock a repository without waiting for another RMCL process.
pub fn try_lock_repository(root: impl AsRef<Path>) -> Result<RepositoryWriteGuard, AppError> {
    let root = root.as_ref();
    std::fs::create_dir_all(root).map_err(|source| AppError::Io {
        operation: "create Minecraft repository for write lock",
        path: root.to_path_buf(),
        source,
    })?;
    let path = root.join(REPOSITORY_LOCK_FILE);
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|source| AppError::Io {
            operation: "open repository write lock",
            path: path.clone(),
            source,
        })?;
    match file.try_lock() {
        Ok(()) => Ok(RepositoryWriteGuard { file, path }),
        Err(TryLockError::WouldBlock) => Err(AppError::RepositoryBusy { path }),
        Err(TryLockError::Error(source)) => Err(AppError::Io {
            operation: "acquire repository write lock",
            path,
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_lock_is_exclusive_and_released_on_drop() {
        let temporary = tempfile::TempDir::new().unwrap();
        let first = try_lock_repository(temporary.path()).unwrap();
        let second_root = temporary.path().join("other");
        let other = try_lock_repository(&second_root).unwrap();
        assert!(matches!(
            try_lock_repository(temporary.path()),
            Err(AppError::RepositoryBusy { .. })
        ));
        drop(first);
        assert!(try_lock_repository(temporary.path()).is_ok());
        drop(other);
    }
}
