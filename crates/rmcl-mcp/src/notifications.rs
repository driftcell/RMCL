use std::collections::BTreeMap;

use rmcl_app::{TaskEvent, TaskId, TaskManager};
use rmcp::{
    ErrorData as McpError, Peer, RoleServer,
    model::{CustomNotification, ServerNotification},
};
use tokio::sync::{Mutex, broadcast};
use tokio_util::sync::CancellationToken;

use crate::{
    error::{internal_error, not_found},
    schema::{TaskNotification, TaskSubscription, TaskSubscriptionEnded, game_log, task_snapshot},
};

pub const TASK_NOTIFICATION_METHOD: &str = "notifications/rmcl/task";

/// Subscriptions owned by one MCP server session.
///
/// Forwarders never own this registry. Dropping the final server reference
/// therefore cancels every forwarder without creating a reference cycle.
pub struct TaskSubscriptions {
    state: Mutex<SubscriptionState>,
}

struct SubscriptionState {
    next_id: u64,
    active: BTreeMap<u64, ActiveSubscription>,
}

struct ActiveSubscription {
    cancellation: CancellationToken,
    worker: tokio::task::JoinHandle<()>,
}

impl Default for TaskSubscriptions {
    fn default() -> Self {
        Self {
            state: Mutex::new(SubscriptionState {
                next_id: 1,
                active: BTreeMap::new(),
            }),
        }
    }
}

impl Drop for TaskSubscriptions {
    fn drop(&mut self) {
        for subscription in self.state.get_mut().active.values() {
            subscription.cancellation.cancel();
        }
    }
}

impl TaskSubscriptions {
    pub async fn subscribe(
        &self,
        tasks: &TaskManager,
        peer: Peer<RoleServer>,
        task_id: Option<u64>,
        include_logs: bool,
    ) -> Result<TaskSubscription, McpError> {
        // Create the receiver before reading current snapshots. Events emitted
        // in between remain queued, so subscribers do not miss that interval.
        let events = tasks.subscribe();
        let task_id = task_id.map(TaskId::from_raw);
        let items = match task_id {
            Some(task_id) => vec![
                tasks
                    .snapshot(task_id)
                    .await
                    .ok_or_else(|| not_found(format!("unknown task ID {}", task_id.get())))?,
            ],
            None => tasks.snapshots().await,
        };
        let subscription_id = {
            let mut state = self.state.lock().await;
            let subscription_id = state.next_id;
            state.next_id = state
                .next_id
                .checked_add(1)
                .expect("MCP task subscription IDs must not overflow");
            let cancellation = CancellationToken::new();
            let worker = tokio::spawn(forward_task_events(
                subscription_id,
                peer,
                events,
                task_id,
                include_logs,
                cancellation.clone(),
            ));
            state.active.insert(
                subscription_id,
                ActiveSubscription {
                    cancellation,
                    worker,
                },
            );
            subscription_id
        };
        Ok(TaskSubscription {
            subscription_id,
            task_id: task_id.map(TaskId::get),
            include_logs,
            items: items.iter().map(task_snapshot).collect(),
        })
    }

    pub async fn unsubscribe(
        &self,
        subscription_id: u64,
    ) -> Result<TaskSubscriptionEnded, McpError> {
        let subscription = self
            .state
            .lock()
            .await
            .active
            .remove(&subscription_id)
            .ok_or_else(|| not_found(format!("unknown task subscription ID {subscription_id}")))?;
        subscription.cancellation.cancel();
        subscription.worker.await.map_err(internal_error)?;
        Ok(TaskSubscriptionEnded {
            subscription_id,
            unsubscribed: true,
        })
    }
}

async fn forward_task_events(
    subscription_id: u64,
    peer: Peer<RoleServer>,
    mut events: broadcast::Receiver<TaskEvent>,
    task_id: Option<TaskId>,
    include_logs: bool,
    cancellation: CancellationToken,
) {
    loop {
        let notification = tokio::select! {
            () = cancellation.cancelled() => break,
            notification = next_notification(
                &mut events,
                subscription_id,
                task_id,
                include_logs,
            ) => notification,
        };
        let Some(notification) = notification else {
            break;
        };
        // TaskManager's broadcast and rmcp's peer queue are both bounded. A
        // slow client can miss events, but can never create an unbounded queue
        // or block task execution; `lagged` tells it to resynchronize.
        let params = match serde_json::to_value(notification) {
            Ok(params) => params,
            Err(error) => {
                eprintln!("RMCL task notification serialization failed: {error}");
                continue;
            }
        };
        let notification = ServerNotification::CustomNotification(CustomNotification::new(
            TASK_NOTIFICATION_METHOD,
            Some(params),
        ));
        let sent = tokio::select! {
            () = cancellation.cancelled() => break,
            result = peer.send_notification(notification) => result,
        };
        if sent.is_err() {
            break;
        }
    }
}

async fn next_notification(
    events: &mut broadcast::Receiver<TaskEvent>,
    subscription_id: u64,
    task_id: Option<TaskId>,
    include_logs: bool,
) -> Option<TaskNotification> {
    loop {
        match events.recv().await {
            Ok(event) => {
                if let Some(notification) =
                    notification_for_event(event, subscription_id, task_id, include_logs)
                {
                    return Some(notification);
                }
            }
            Err(broadcast::error::RecvError::Lagged(missed)) => {
                return Some(TaskNotification::Lagged {
                    subscription_id,
                    missed,
                });
            }
            Err(broadcast::error::RecvError::Closed) => return None,
        }
    }
}

fn notification_for_event(
    event: TaskEvent,
    subscription_id: u64,
    task_id: Option<TaskId>,
    include_logs: bool,
) -> Option<TaskNotification> {
    match event {
        TaskEvent::Snapshot(snapshot) if selected(task_id, snapshot.id) => {
            Some(TaskNotification::Snapshot {
                subscription_id,
                task: task_snapshot(&snapshot),
            })
        }
        TaskEvent::GameLog {
            task_id: event_task_id,
            line,
            log,
        } if include_logs && selected(task_id, event_task_id) => Some(TaskNotification::Log {
            subscription_id,
            task_id: event_task_id.get(),
            line,
            log: game_log(&log),
        }),
        TaskEvent::Dismissed(event_task_id) if selected(task_id, event_task_id) => {
            Some(TaskNotification::Dismissed {
                subscription_id,
                task_id: event_task_id.get(),
            })
        }
        _ => None,
    }
}

fn selected(filter: Option<TaskId>, candidate: TaskId) -> bool {
    filter.is_none_or(|filter| filter == candidate)
}

#[cfg(test)]
mod tests {
    use rmcl_app::{TaskKind, TaskSnapshot, TaskStatus};
    use rmcl_launch::{GameLog, LogStream};
    use serde_json::json;

    use super::*;

    fn snapshot(id: u64) -> TaskSnapshot {
        TaskSnapshot {
            id: TaskId::from_raw(id),
            kind: TaskKind::VanillaInstall,
            subject: None,
            status: TaskStatus::Running,
            phase: None,
            progress: None,
            error: None,
            exit: None,
            stop_requested: false,
            log_lines: 0,
        }
    }

    #[test]
    fn event_notifications_are_typed_and_filtered() {
        let snapshot_notification = notification_for_event(
            TaskEvent::Snapshot(snapshot(7)),
            3,
            Some(TaskId::from_raw(7)),
            false,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(snapshot_notification).unwrap(),
            json!({
                "type": "snapshot",
                "subscription_id": 3,
                "task": {
                    "id": 7,
                    "kind": "vanilla_install",
                    "subject": null,
                    "status": "running",
                    "phase": null,
                    "progress": null,
                    "error": null,
                    "exit": null,
                    "stop_requested": false,
                    "log_lines": 0,
                }
            })
        );
        assert!(
            notification_for_event(
                TaskEvent::Snapshot(snapshot(8)),
                3,
                Some(TaskId::from_raw(7)),
                false,
            )
            .is_none()
        );
        let log = TaskEvent::GameLog {
            task_id: TaskId::from_raw(7),
            line: 2,
            log: GameLog {
                stream: LogStream::Stderr,
                text: "redacted".to_owned(),
            },
        };
        assert!(notification_for_event(log.clone(), 3, None, false).is_none());
        assert_eq!(
            serde_json::to_value(notification_for_event(log, 3, None, true).unwrap()).unwrap(),
            json!({
                "type": "log",
                "subscription_id": 3,
                "task_id": 7,
                "line": 2,
                "log": {"stream": "stderr", "text": "redacted"}
            })
        );
        assert_eq!(
            serde_json::to_value(
                notification_for_event(TaskEvent::Dismissed(TaskId::from_raw(7)), 3, None, false)
                    .unwrap()
            )
            .unwrap(),
            json!({"type": "dismissed", "subscription_id": 3, "task_id": 7})
        );
    }

    #[tokio::test]
    async fn unknown_unsubscribe_has_a_stable_not_found_error() {
        let subscriptions = TaskSubscriptions::default();
        let error = subscriptions.unsubscribe(99).await.unwrap_err();
        assert_eq!(error.data.as_ref().unwrap()["code"], "not_found");
        assert_eq!(error.data.as_ref().unwrap()["retryable"], false);
    }

    #[tokio::test]
    async fn overflow_reports_lagged_and_filtered_logs_are_skipped() {
        let (sender, mut receiver) = broadcast::channel(1);
        sender.send(TaskEvent::Snapshot(snapshot(1))).unwrap();
        sender.send(TaskEvent::Snapshot(snapshot(2))).unwrap();
        let lagged = next_notification(&mut receiver, 4, None, false)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(lagged).unwrap(),
            json!({"type": "lagged", "subscription_id": 4, "missed": 1})
        );

        let (sender, mut receiver) = broadcast::channel(4);
        sender
            .send(TaskEvent::GameLog {
                task_id: TaskId::from_raw(7),
                line: 1,
                log: GameLog {
                    stream: LogStream::Stdout,
                    text: "hidden without log subscription".to_owned(),
                },
            })
            .unwrap();
        sender.send(TaskEvent::Snapshot(snapshot(7))).unwrap();
        let snapshot = next_notification(&mut receiver, 4, None, false)
            .await
            .unwrap();
        assert!(matches!(snapshot, TaskNotification::Snapshot { .. }));
    }
}
