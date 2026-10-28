//! UI-facing events for one assistant turn.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use tokio::sync::{Mutex, mpsc, oneshot};

/// One user-visible confirmation request.
#[derive(Debug, Clone)]
pub struct ConfirmationRequest {
    pub tool: String,
    pub title: String,
    pub body: String,
    pub confirm_label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationDecision {
    Approve,
    Reject,
    Stop,
}

/// Events consumed by the assistant page.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    Started,
    Status {
        message: String,
        detail: Option<String>,
    },
    ToolStarted {
        id: u64,
        name: String,
        label: String,
        summary: String,
        task_id: Option<u64>,
    },
    ToolFinished {
        id: u64,
        name: String,
        success: bool,
        skipped: bool,
        error: Option<String>,
        task_id: Option<u64>,
    },
    Confirmation {
        request_id: u64,
        request: ConfirmationRequest,
    },
    Completed {
        markdown: String,
    },
    Failed {
        message: String,
    },
    Stopped,
}

/// Asynchronous bridge between a Rig tool hook and the GPUI page.
#[derive(Clone, Default)]
pub struct ConfirmationBroker {
    next: Arc<AtomicU64>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<ConfirmationDecision>>>>,
}

impl ConfirmationBroker {
    pub async fn ask(
        &self,
        events: &mpsc::UnboundedSender<AgentEvent>,
        request: ConfirmationRequest,
    ) -> ConfirmationDecision {
        let request_id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(request_id, sender);
        if events
            .send(AgentEvent::Confirmation {
                request_id,
                request,
            })
            .is_err()
        {
            self.pending.lock().await.remove(&request_id);
            return ConfirmationDecision::Reject;
        }
        tokio::select! {
            decision = receiver => decision.unwrap_or(ConfirmationDecision::Reject),
            _ = tokio::time::sleep(Duration::from_secs(600)) => {
                self.pending.lock().await.remove(&request_id);
                ConfirmationDecision::Reject
            }
        }
    }

    pub async fn resolve(&self, request_id: u64, decision: ConfirmationDecision) {
        let sender = self.pending.lock().await.remove(&request_id);
        if let Some(sender) = sender {
            let _ = sender.send(decision);
        }
    }
}

/// The receiving side of one turn. Kept by the UI while it renders updates.
pub struct AgentTurn {
    pub events: mpsc::UnboundedReceiver<AgentEvent>,
    pub confirmations: ConfirmationBroker,
    cancellation: tokio_util::sync::CancellationToken,
    handle: tokio::task::JoinHandle<()>,
}

impl AgentTurn {
    pub(crate) fn new(
        events: mpsc::UnboundedReceiver<AgentEvent>,
        confirmations: ConfirmationBroker,
        cancellation: tokio_util::sync::CancellationToken,
        handle: tokio::task::JoinHandle<()>,
    ) -> Self {
        Self {
            events,
            confirmations,
            cancellation,
            handle,
        }
    }

    pub fn into_parts(self) -> (mpsc::UnboundedReceiver<AgentEvent>, AgentTurnControl) {
        (
            self.events,
            AgentTurnControl {
                confirmations: self.confirmations,
                cancellation: self.cancellation,
                handle: self.handle,
            },
        )
    }
}

/// The host-side handle for an active turn.
pub struct AgentTurnControl {
    pub confirmations: ConfirmationBroker,
    cancellation: tokio_util::sync::CancellationToken,
    handle: tokio::task::JoinHandle<()>,
}

impl AgentTurnControl {
    pub fn stop(&self) {
        self.cancellation.cancel();
        self.handle.abort();
    }

    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
}
