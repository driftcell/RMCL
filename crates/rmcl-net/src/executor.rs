use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
    time::Duration,
};

use futures::{StreamExt, stream};
use reqwest::StatusCode;
use rmcl_domain::{DownloadRequest, Sha1Hash};
use sha1::{Digest, Sha1};
use tempfile::NamedTempFile;
use tokio::{
    fs,
    io::AsyncWriteExt,
    sync::{Mutex, Semaphore},
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::{
    DownloadConfig, DownloadError, DownloadFailure, DownloadOutcome, DownloadReport,
    DownloadStatus, ProgressEvent, ProgressSender, ProgressStage, SourceFailure,
    integrity::{io_error, verified_existing_size},
};

#[derive(Clone)]
pub struct DownloadExecutor {
    client: reqwest::Client,
    config: DownloadConfig,
    host_permits: Arc<Mutex<HashMap<String, Arc<Semaphore>>>>,
}

impl DownloadExecutor {
    pub fn new(config: DownloadConfig) -> Result<Self, DownloadError> {
        if config.max_concurrency == 0
            || config.max_concurrency_per_host == 0
            || config.max_attempts_per_source == 0
        {
            return Err(DownloadError::InvalidConfiguration);
        }
        let client = reqwest::Client::builder()
            .user_agent(concat!("RMCL/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()?;
        Ok(Self {
            client,
            config,
            host_permits: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub async fn download_file(
        &self,
        request: &DownloadRequest,
        cancellation: CancellationToken,
        progress: Option<ProgressSender>,
    ) -> Result<DownloadStatus, DownloadError> {
        let result = self
            .download_file_inner(request, &cancellation, progress.clone())
            .await;
        if let Some(progress) = progress {
            let (stage, bytes, detail) = match &result {
                Ok(DownloadStatus::Downloaded { bytes }) => {
                    (ProgressStage::Completed, *bytes, None)
                }
                Ok(DownloadStatus::Skipped { bytes }) => (ProgressStage::Skipped, *bytes, None),
                Err(DownloadError::Cancelled) => (ProgressStage::Cancelled, 0, None),
                Err(error) => (ProgressStage::Failed, 0, Some(error.to_string())),
            };
            progress
                .report(ProgressEvent {
                    destination: request.destination.clone(),
                    stage,
                    downloaded_bytes: bytes,
                    total_bytes: request.size,
                    detail,
                })
                .await;
        }
        result
    }

    pub async fn download_all(
        &self,
        requests: Vec<DownloadRequest>,
        cancellation: CancellationToken,
        progress: Option<ProgressSender>,
    ) -> DownloadReport {
        let mut report = DownloadReport::default();
        let mut destinations = HashSet::new();
        let mut unique_requests = Vec::new();
        for request in requests {
            if destinations.insert(request.destination.clone()) {
                unique_requests.push(request);
            } else {
                let destination = request.destination.display().to_string();
                report.failures.push(DownloadFailure {
                    request,
                    error: DownloadError::ConflictingDestination(destination),
                });
            }
        }

        let mut results = stream::iter(unique_requests.into_iter().map(|request| {
            let cancellation = cancellation.clone();
            let progress = progress.clone();
            async move {
                match self.download_file(&request, cancellation, progress).await {
                    Ok(status) => Ok(DownloadOutcome { request, status }),
                    Err(error) => Err(DownloadFailure { request, error }),
                }
            }
        }))
        .buffer_unordered(self.config.max_concurrency);

        while let Some(result) = results.next().await {
            let cancelled = matches!(
                &result,
                Err(DownloadFailure {
                    error: DownloadError::Cancelled,
                    ..
                })
            );
            match result {
                Ok(outcome) => report.outcomes.push(outcome),
                Err(failure) => report.failures.push(failure),
            }
            if cancelled {
                report.cancelled = true;
                break;
            }
        }

        report
            .outcomes
            .sort_by(|left, right| left.request.destination.cmp(&right.request.destination));
        report
            .failures
            .sort_by(|left, right| left.request.destination.cmp(&right.request.destination));
        report
    }

    async fn download_file_inner(
        &self,
        request: &DownloadRequest,
        cancellation: &CancellationToken,
        progress: Option<ProgressSender>,
    ) -> Result<DownloadStatus, DownloadError> {
        if request.sources.is_empty() {
            return Err(DownloadError::EmptySources);
        }
        if request.destination.as_os_str().is_empty() {
            return Err(DownloadError::InvalidDestination(String::new()));
        }
        report_progress(&progress, request, ProgressStage::Checking, 0, None).await;

        if let Some(bytes) = verified_existing_size(request).await? {
            return Ok(DownloadStatus::Skipped { bytes });
        }

        let parent = parent_directory(&request.destination);
        fs::create_dir_all(parent)
            .await
            .map_err(|error| io_error(parent, error))?;

        let mut failures = Vec::new();
        for (source_index, source) in request.sources.iter().enumerate() {
            let source_label = redacted_url(source);
            if !matches!(source.scheme(), "http" | "https") {
                failures.push(SourceFailure {
                    source: source_label,
                    reason: format!("unsupported URL scheme `{}`", source.scheme()),
                });
                continue;
            }

            // Earlier candidates are preferred mirrors. Try them once so a
            // dead mirror cannot delay the official fallback; the last source
            // keeps the configured retry budget.
            let max_attempts = if source_index + 1 == request.sources.len() {
                self.config.max_attempts_per_source
            } else {
                1
            };
            for attempt in 1..=max_attempts {
                if cancellation.is_cancelled() {
                    return Err(DownloadError::Cancelled);
                }
                report_progress(
                    &progress,
                    request,
                    ProgressStage::Downloading,
                    0,
                    Some(format!("{source_label} (attempt {attempt})")),
                )
                .await;

                let result = tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => return Err(DownloadError::Cancelled),
                    result = self.download_to_temp(request, source, parent, &progress) => result,
                };
                match result {
                    Ok((temporary, bytes)) => {
                        if cancellation.is_cancelled() {
                            drop(temporary);
                            return Err(DownloadError::Cancelled);
                        }
                        temporary
                            .persist(&request.destination)
                            .map_err(|error| io_error(&request.destination, error.error))?;
                        return Ok(DownloadStatus::Downloaded { bytes });
                    }
                    Err(AttemptError::Fatal(error)) => return Err(error),
                    Err(AttemptError::Source { failure, retryable }) => {
                        failures.push(failure);
                        if retryable && attempt < max_attempts {
                            let delay = self.retry_delay(attempt);
                            tokio::select! {
                                biased;
                                _ = cancellation.cancelled() => return Err(DownloadError::Cancelled),
                                _ = tokio::time::sleep(delay) => {}
                            }
                        } else {
                            break;
                        }
                    }
                }
            }
        }

        Err(DownloadError::Exhausted {
            destination: request.destination.display().to_string(),
            failures,
        })
    }

    async fn download_to_temp(
        &self,
        request: &DownloadRequest,
        source: &Url,
        parent: &Path,
        progress: &Option<ProgressSender>,
    ) -> Result<(NamedTempFile, u64), AttemptError> {
        let host = host_key(source);
        let semaphore = {
            let mut host_permits = self.host_permits.lock().await;
            host_permits
                .entry(host)
                .or_insert_with(|| Arc::new(Semaphore::new(self.config.max_concurrency_per_host)))
                .clone()
        };
        let _permit = semaphore
            .acquire()
            .await
            .map_err(|_| AttemptError::Fatal(DownloadError::ConcurrencyLimiterClosed))?;

        let mut response = self
            .client
            .get(source.clone())
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(|error| network_failure(source, &error))?;

        let status = response.status();
        if !status.is_success() {
            return Err(source_failure(
                source,
                format!("HTTP {}", status.as_u16()),
                is_retryable_status(status),
            ));
        }
        if let Some(encoding) = response
            .headers()
            .get(reqwest::header::CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.eq_ignore_ascii_case("identity"))
        {
            return Err(source_failure(
                source,
                format!("unsupported content encoding `{encoding}`"),
                false,
            ));
        }

        let response_length = response.content_length();
        let temporary = NamedTempFile::new_in(parent)
            .map_err(|error| AttemptError::Fatal(io_error(parent, error)))?;
        let temporary_path = temporary.path().to_path_buf();
        let mut file = fs::File::from_std(
            temporary
                .reopen()
                .map_err(|error| AttemptError::Fatal(io_error(&temporary_path, error)))?,
        );
        let mut hasher = request.sha1.map(|_| Sha1::new());
        let mut downloaded = 0_u64;
        let mut last_progress = Instant::now();

        loop {
            let chunk = response
                .chunk()
                .await
                .map_err(|error| network_failure(source, &error))?;
            let Some(chunk) = chunk else {
                break;
            };
            downloaded += chunk.len() as u64;
            if let Some(expected) = request.size
                && downloaded > expected
            {
                return Err(source_failure(
                    source,
                    format!("downloaded {downloaded} bytes, expected {expected}"),
                    true,
                ));
            }
            file.write_all(&chunk)
                .await
                .map_err(|error| AttemptError::Fatal(io_error(&temporary_path, error)))?;
            if let Some(hasher) = &mut hasher {
                hasher.update(&chunk);
            }

            if last_progress.elapsed() >= Duration::from_millis(100) {
                report_progress(
                    progress,
                    request,
                    ProgressStage::Downloading,
                    downloaded,
                    Some(redacted_url(source)),
                )
                .await;
                last_progress = Instant::now();
            }
        }

        if let Some(expected) = response_length
            && expected != downloaded
        {
            return Err(source_failure(
                source,
                format!("response ended at {downloaded} bytes, content-length was {expected}"),
                true,
            ));
        }
        if let Some(expected) = request.size
            && expected != downloaded
        {
            return Err(source_failure(
                source,
                format!("downloaded {downloaded} bytes, expected {expected}"),
                true,
            ));
        }
        if let Some(expected) = request.sha1 {
            let Some(hasher) = hasher else {
                return Err(source_failure(source, "missing SHA-1 state", false));
            };
            let digest = hasher.finalize();
            let mut bytes = [0_u8; 20];
            bytes.copy_from_slice(&digest);
            let actual = Sha1Hash::from_bytes(bytes);
            if actual != expected {
                return Err(source_failure(
                    source,
                    format!("SHA-1 mismatch: expected {expected}, got {actual}"),
                    true,
                ));
            }
        }

        report_progress(
            progress,
            request,
            ProgressStage::Verifying,
            downloaded,
            None,
        )
        .await;
        file.sync_all()
            .await
            .map_err(|error| AttemptError::Fatal(io_error(&temporary_path, error)))?;
        drop(file);
        Ok((temporary, downloaded))
    }

    fn retry_delay(&self, attempt: usize) -> Duration {
        let factor = 1_u32 << attempt.saturating_sub(1).min(5);
        let delay = self
            .config
            .initial_retry_delay
            .saturating_mul(factor)
            .min(self.config.max_retry_delay);
        let jitter_bound = (delay.as_millis().min(100) as u64) / 10;
        if jitter_bound == 0 {
            delay
        } else {
            delay + Duration::from_millis(fastrand::u64(0..=jitter_bound))
        }
    }
}

fn host_key(source: &Url) -> String {
    format!(
        "{}://{}:{}",
        source.scheme(),
        source.host_str().unwrap_or(""),
        source.port_or_known_default().unwrap_or(0)
    )
}

fn parent_directory(destination: &Path) -> &Path {
    destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn is_retryable_status(status: StatusCode) -> bool {
    status.is_server_error()
        || matches!(
            status,
            StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
        )
}

fn network_failure(source: &Url, error: &reqwest::Error) -> AttemptError {
    let reason = if error.is_timeout() {
        "request timed out"
    } else if error.is_connect() {
        "connection failed"
    } else if error.is_body() {
        "response body failed"
    } else {
        "request failed"
    };
    source_failure(source, reason, true)
}

fn source_failure(source: &Url, reason: impl Into<String>, retryable: bool) -> AttemptError {
    AttemptError::Source {
        failure: SourceFailure {
            source: redacted_url(source),
            reason: reason.into(),
        },
        retryable,
    }
}

fn redacted_url(source: &Url) -> String {
    let mut redacted = source.clone();
    let _ = redacted.set_username("");
    let _ = redacted.set_password(None);
    redacted.set_query(None);
    redacted.set_fragment(None);
    redacted.to_string()
}

async fn report_progress(
    progress: &Option<ProgressSender>,
    request: &DownloadRequest,
    stage: ProgressStage,
    downloaded_bytes: u64,
    detail: Option<String>,
) {
    if let Some(progress) = progress {
        progress
            .report(ProgressEvent {
                destination: request.destination.clone(),
                stage,
                downloaded_bytes,
                total_bytes: request.size,
                detail,
            })
            .await;
    }
}

enum AttemptError {
    Source {
        failure: SourceFailure,
        retryable: bool,
    },
    Fatal(DownloadError),
}
