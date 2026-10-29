//! UI-facing events for one assistant turn.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use rig::message::Message;
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
    ConfirmationExpired {
        request_id: u64,
    },
    Completed {
        markdown: String,
        messages: Vec<Message>,
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
    request_lock: Arc<Mutex<()>>,
}

impl ConfirmationBroker {
    pub async fn ask(
        &self,
        events: &mpsc::UnboundedSender<AgentEvent>,
        request: ConfirmationRequest,
    ) -> ConfirmationDecision {
        // The page presents one decision at a time. Parallel tool calls may
        // still run, but their confirmation requests must not replace each
        // other in the UI.
        let _request_guard = self.request_lock.lock().await;
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
                let _ = events.send(AgentEvent::ConfirmationExpired { request_id });
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

    pub fn into_parts(
        self,
    ) -> (
        mpsc::UnboundedReceiver<AgentEvent>,
        AgentTurnControl,
        tokio::task::JoinHandle<()>,
    ) {
        let abort_handle = self.handle.abort_handle();
        (
            self.events,
            AgentTurnControl {
                confirmations: self.confirmations,
                cancellation: self.cancellation,
                abort_handle,
            },
            self.handle,
        )
    }
}

/// The host-side handle for an active turn.
pub struct AgentTurnControl {
    pub confirmations: ConfirmationBroker,
    cancellation: tokio_util::sync::CancellationToken,
    abort_handle: tokio::task::AbortHandle,
}

impl AgentTurnControl {
    pub fn stop(&self) {
        self.cancellation.cancel();
        self.abort_handle.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(tool: &str) -> ConfirmationRequest {
        ConfirmationRequest {
            tool: tool.to_owned(),
            title: tool.to_owned(),
            body: String::new(),
            confirm_label: "继续".to_owned(),
        }
    }

    #[tokio::test]
    async fn parallel_confirmation_requests_are_presented_one_at_a_time() {
        let broker = ConfirmationBroker::default();
        let (events, mut receiver) = mpsc::unbounded_channel();

        let first = {
            let broker = broker.clone();
            let events = events.clone();
            tokio::spawn(async move { broker.ask(&events, request("first")).await })
        };
        let Some(AgentEvent::Confirmation {
            request_id: first_id,
            ..
        }) = receiver.recv().await
        else {
            panic!("first confirmation was not emitted");
        };

        let second = {
            let broker = broker.clone();
            let events = events.clone();
            tokio::spawn(async move { broker.ask(&events, request("second")).await })
        };
        tokio::task::yield_now().await;
        assert!(receiver.try_recv().is_err());

        broker
            .resolve(first_id, ConfirmationDecision::Approve)
            .await;
        assert_eq!(first.await.unwrap(), ConfirmationDecision::Approve);

        let Some(AgentEvent::Confirmation {
            request_id: second_id,
            request,
            ..
        }) = receiver.recv().await
        else {
            panic!("second confirmation was not emitted after resolving the first");
        };
        assert_eq!(request.tool, "second");
        broker
            .resolve(second_id, ConfirmationDecision::Reject)
            .await;
        assert_eq!(second.await.unwrap(), ConfirmationDecision::Reject);
    }

    #[tokio::test(start_paused = true)]
    async fn expired_confirmation_is_reported_to_the_page() {
        let broker = ConfirmationBroker::default();
        let (events, mut receiver) = mpsc::unbounded_channel();
        let pending = {
            let broker = broker.clone();
            let events = events.clone();
            tokio::spawn(async move { broker.ask(&events, request("delete")).await })
        };
        let Some(AgentEvent::Confirmation { request_id, .. }) = receiver.recv().await else {
            panic!("confirmation was not emitted");
        };

        tokio::time::advance(Duration::from_secs(600)).await;

        assert_eq!(pending.await.unwrap(), ConfirmationDecision::Reject);
        assert!(matches!(
            receiver.recv().await,
            Some(AgentEvent::ConfirmationExpired {
                request_id: expired_id,
            }) if expired_id == request_id
        ));
    }
}
