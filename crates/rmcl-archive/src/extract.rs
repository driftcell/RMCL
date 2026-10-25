use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
};

use crc32fast::Hasher;
use rmcl_domain::ExtractionPlan;
use tempfile::Builder;
use tokio_util::sync::CancellationToken;
use zip::ZipArchive;

use crate::{
    ExtractionError, ExtractionLimits, ExtractionProgress, ExtractionReport, path::EntryPath,
};

#[derive(Debug, Clone, Default)]
pub struct ArchiveExtractor {
    limits: ExtractionLimits,
}

impl ArchiveExtractor {
    pub fn new(limits: ExtractionLimits) -> Self {
        Self { limits }
    }

    /// Extracts a ZIP archive synchronously and publishes only a completely validated new directory.
    pub fn extract_zip(
        &self,
        request: &ExtractionPlan,
        cancellation: CancellationToken,
        mut progress: Option<&mut dyn FnMut(ExtractionProgress)>,
    ) -> Result<ExtractionReport, ExtractionError> {
        validate_request(request)?;
        ensure_destination_available(&request.destination)?;
        let parent = parent_directory(&request.destination);
        fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
        let staging = Builder::new()
            .prefix(".rmcl-extract-")
            .tempdir_in(parent)
            .map_err(|error| io_error(parent, error))?;
        let staging_root = staging.path().join("root");
        fs::create_dir(&staging_root).map_err(|error| io_error(&staging_root, error))?;

        let source =
            fs::File::open(&request.source).map_err(|error| io_error(&request.source, error))?;
        let mut archive = ZipArchive::new(source).map_err(|source| ExtractionError::Zip {
            path: request.source.display().to_string(),
            source,
        })?;
        let total_entries = archive.len();
        if total_entries > self.limits.max_entries {
            return Err(ExtractionError::TooManyEntries {
                actual: total_entries,
                limit: self.limits.max_entries,
            });
        }

        let mut report = ExtractionReport {
            destination: request.destination.clone(),
            ..ExtractionReport::default()
        };
        let mut seen = HashSet::new();
        let mut declared_total = 0_u64;

        for index in 0..total_entries {
            if cancellation.is_cancelled() {
                return Err(ExtractionError::Cancelled);
            }
            let mut entry = archive
                .by_index(index)
                .map_err(|source| ExtractionError::Zip {
                    path: request.source.display().to_string(),
                    source,
                })?;
            let name = entry
                .name()
                .map_err(|source| ExtractionError::Zip {
                    path: request.source.display().to_string(),
                    source,
                })?
                .into_owned();
            let entry_path = EntryPath::new(&name)?;

            if request
                .exclude_prefixes
                .iter()
                .any(|prefix| name.starts_with(prefix))
            {
                report.skipped += 1;
                notify(
                    &mut progress,
                    name,
                    index + 1,
                    total_entries,
                    report.extracted_bytes,
                );
                continue;
            }
            if entry.is_symlink() || !supported_entry_type(&entry) {
                return Err(ExtractionError::UnsupportedEntry { entry: name });
            }
            if !seen.insert(entry_path.collision_key().to_owned()) {
                return Err(ExtractionError::DuplicateEntry { entry: name });
            }

            let target = entry_path.to_path(&staging_root);
            if entry.is_dir() {
                fs::create_dir_all(&target).map_err(|error| io_error(&target, error))?;
                report.directories += 1;
            } else {
                if entry.size() > self.limits.max_entry_size {
                    return Err(ExtractionError::EntryTooLarge {
                        entry: name,
                        actual: entry.size(),
                        limit: self.limits.max_entry_size,
                    });
                }
                declared_total = declared_total
                    .checked_add(entry.size())
                    .filter(|total| *total <= self.limits.max_total_size)
                    .ok_or(ExtractionError::TotalTooLarge {
                        actual: declared_total.saturating_add(entry.size()),
                        limit: self.limits.max_total_size,
                    })?;

                let unix_mode = entry.unix_mode();
                let expected_size = entry.size();
                let expected_crc = entry.crc32();
                let extracted = write_entry(
                    &mut entry,
                    &name,
                    expected_size,
                    expected_crc,
                    &target,
                    report.extracted_bytes,
                    &self.limits,
                    &cancellation,
                )?;
                apply_permissions(&target, unix_mode).map_err(|error| io_error(&target, error))?;
                report.extracted_bytes = report.extracted_bytes.checked_add(extracted).ok_or(
                    ExtractionError::TotalTooLarge {
                        actual: u64::MAX,
                        limit: self.limits.max_total_size,
                    },
                )?;
                report.files += 1;
            }

            notify(
                &mut progress,
                name,
                index + 1,
                total_entries,
                report.extracted_bytes,
            );
        }

        if cancellation.is_cancelled() {
            return Err(ExtractionError::Cancelled);
        }
        drop(archive);
        ensure_destination_available(&request.destination)?;
        fs::rename(&staging_root, &request.destination)
            .map_err(|error| io_error(&request.destination, error))?;
        drop(staging);
        Ok(report)
    }
}

#[allow(clippy::too_many_arguments)]
fn write_entry(
    reader: &mut dyn Read,
    name: &str,
    expected_size: u64,
    expected_crc: u32,
    target: &Path,
    already_extracted: u64,
    limits: &ExtractionLimits,
    cancellation: &CancellationToken,
) -> Result<u64, ExtractionError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|error| io_error(target, error))?;
    let mut hasher = Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut written = 0_u64;

    loop {
        if cancellation.is_cancelled() {
            return Err(ExtractionError::Cancelled);
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|source| ExtractionError::Io {
                path: name.to_owned(),
                source,
            })?;
        if read == 0 {
            break;
        }
        let next_written =
            written
                .checked_add(read as u64)
                .ok_or(ExtractionError::EntryTooLarge {
                    entry: name.to_owned(),
                    actual: u64::MAX,
                    limit: limits.max_entry_size,
                })?;
        if next_written > limits.max_entry_size {
            return Err(ExtractionError::EntryTooLarge {
                entry: name.to_owned(),
                actual: next_written,
                limit: limits.max_entry_size,
            });
        }
        let next_total =
            already_extracted
                .checked_add(next_written)
                .ok_or(ExtractionError::TotalTooLarge {
                    actual: u64::MAX,
                    limit: limits.max_total_size,
                })?;
        if next_total > limits.max_total_size {
            return Err(ExtractionError::TotalTooLarge {
                actual: next_total,
                limit: limits.max_total_size,
            });
        }

        file.write_all(&buffer[..read])
            .map_err(|error| io_error(target, error))?;
        hasher.update(&buffer[..read]);
        written = next_written;
    }

    if written != expected_size {
        return Err(ExtractionError::EntrySizeMismatch {
            entry: name.to_owned(),
            expected: expected_size,
            actual: written,
        });
    }
    if hasher.finalize() != expected_crc {
        return Err(ExtractionError::CrcMismatch {
            entry: name.to_owned(),
        });
    }
    file.flush().map_err(|error| io_error(target, error))?;
    drop(file);
    Ok(written)
}

fn validate_request(request: &ExtractionPlan) -> Result<(), ExtractionError> {
    if request.destination.as_os_str().is_empty() {
        return Err(ExtractionError::InvalidDestination(
            "destination is empty".to_owned(),
        ));
    }
    let metadata =
        fs::metadata(&request.source).map_err(|error| io_error(&request.source, error))?;
    if !metadata.is_file() {
        return Err(ExtractionError::InvalidSource(
            request.source.display().to_string(),
        ));
    }
    Ok(())
}

fn ensure_destination_available(destination: &Path) -> Result<(), ExtractionError> {
    match fs::symlink_metadata(destination) {
        Ok(_) => Err(ExtractionError::DestinationExists(
            destination.display().to_string(),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(destination, error)),
    }
}

fn supported_entry_type<R: Read>(entry: &zip::read::ZipFile<'_, R>) -> bool {
    // NOTE: System::Dos entries can still carry Unix types in the raw external attributes.
    let raw_type = (entry.external_attributes() >> 16) & 0o170000;
    if raw_type == 0o120000 || entry.is_symlink() {
        return false;
    }
    let file_type = if raw_type == 0 {
        entry.unix_mode().map(|mode| mode & 0o170000)
    } else {
        Some(raw_type)
    };
    if entry.is_dir() {
        matches!(file_type, None | Some(0) | Some(0o040000))
    } else if entry.is_file() {
        matches!(file_type, None | Some(0) | Some(0o100000))
    } else {
        false
    }
}

fn parent_directory(destination: &Path) -> &Path {
    destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn notify(
    progress: &mut Option<&mut dyn FnMut(ExtractionProgress)>,
    entry: String,
    completed_entries: usize,
    total_entries: usize,
    extracted_bytes: u64,
) {
    if let Some(progress) = progress.as_deref_mut() {
        progress(ExtractionProgress {
            entry,
            completed_entries,
            total_entries,
            extracted_bytes,
        });
    }
}

fn io_error(path: &Path, source: std::io::Error) -> ExtractionError {
    ExtractionError::Io {
        path: path.display().to_string(),
        source,
    }
}

#[cfg(unix)]
fn apply_permissions(path: &Path, mode: Option<u32>) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    if let Some(mode) = mode {
        let mode = mode & 0o777;
        if mode != 0 {
            fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn apply_permissions(_path: &Path, _mode: Option<u32>) -> std::io::Result<()> {
    Ok(())
}
