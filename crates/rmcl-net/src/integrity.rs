use std::path::Path;

use rmcl_domain::{DownloadRequest, Sha1Hash};
use sha1::{Digest, Sha1};
use tokio::{fs, io::AsyncReadExt};

use crate::DownloadError;

pub(crate) async fn verified_existing_size(
    request: &DownloadRequest,
) -> Result<Option<u64>, DownloadError> {
    let path = &request.destination;
    let metadata = match fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(path, error)),
    };
    if !metadata.file_type().is_file() {
        return Err(DownloadError::InvalidDestination(
            path.display().to_string(),
        ));
    }

    if request
        .size
        .is_some_and(|expected| expected != metadata.len())
    {
        return Ok(None);
    }
    if let Some(expected) = request.sha1 {
        let actual = hash_file(path).await?;
        if actual != expected {
            return Ok(None);
        }
    }

    if request.sha1.is_none() && request.size.is_none() {
        return Ok(None);
    }
    Ok(Some(metadata.len()))
}

async fn hash_file(path: &Path) -> Result<Sha1Hash, DownloadError> {
    let mut file = fs::File::open(path)
        .await
        .map_err(|error| io_error(path, error))?;
    let mut hasher = Sha1::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|error| io_error(path, error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    let digest = hasher.finalize();
    let mut bytes = [0_u8; 20];
    bytes.copy_from_slice(&digest);
    Ok(Sha1Hash::from_bytes(bytes))
}

pub(crate) fn io_error(path: &Path, source: std::io::Error) -> DownloadError {
    DownloadError::Io {
        path: path.display().to_string(),
        source,
    }
}
