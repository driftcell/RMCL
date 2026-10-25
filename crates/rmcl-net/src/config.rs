use std::time::Duration;

#[derive(Debug, Clone)]
pub struct DownloadConfig {
    pub max_concurrency: usize,
    pub max_concurrency_per_host: usize,
    pub max_attempts_per_source: usize,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub initial_retry_delay: Duration,
    pub max_retry_delay: Duration,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            max_concurrency: 8,
            max_concurrency_per_host: 4,
            max_attempts_per_source: 3,
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(300),
            initial_retry_delay: Duration::from_millis(200),
            max_retry_delay: Duration::from_secs(5),
        }
    }
}
