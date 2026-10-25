use std::path::PathBuf;

use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressEvent {
    pub destination: PathBuf,
    pub stage: ProgressStage,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressStage {
    Checking,
    Downloading,
    Verifying,
    Completed,
    Skipped,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct ProgressSender {
    sender: mpsc::Sender<ProgressEvent>,
}

impl ProgressSender {
    pub(crate) async fn report(&self, event: ProgressEvent) {
        if self.sender.send(event).await.is_err() {
            // A closed progress receiver must not fail an otherwise valid download.
        }
    }
}

pub fn progress_channel(capacity: usize) -> (ProgressSender, mpsc::Receiver<ProgressEvent>) {
    let (sender, receiver) = mpsc::channel(capacity.max(1));
    (ProgressSender { sender }, receiver)
}
