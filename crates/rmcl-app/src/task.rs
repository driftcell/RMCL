use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use rmcl_instance::VersionId;
use tokio::sync::{Mutex, broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::{
    AppError, InstallDownloadProgress, InstallEvent, InstallPhase, VanillaInstallRequest,
    VanillaInstaller,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskId(u64);

impl TaskId {
    pub fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    VanillaInstall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Running,
    Cancelling,
    Completed,
    Cancelled,
    Failed,
}

impl TaskStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskProgress {
    Download(InstallDownloadProgress),
    Natives {
        current: PathBuf,
        completed_archives: usize,
        total_archives: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSnapshot {
    pub id: TaskId,
    pub kind: TaskKind,
    pub status: TaskStatus,
    pub phase: Option<InstallPhase>,
    pub progress: Option<TaskProgress>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum TaskEvent {
    Snapshot(TaskSnapshot),
    Dismissed(TaskId),
}

#[derive(Clone)]
pub struct TaskManager {
    inner: Arc<TaskInner>,
}

struct TaskInner {
    installer: VanillaInstaller,
    next_id: AtomicU64,
    registry: Mutex<Registry>,
    events: broadcast::Sender<TaskEvent>,
}

#[derive(Default)]
struct Registry {
    tasks: TaskMap,
    active_installs: Vec<ActiveInstall>,
}

type TaskMap = std::collections::HashMap<TaskId, TaskRecord>;

struct TaskRecord {
    snapshot: TaskSnapshot,
    cancellation: CancellationToken,
}

#[derive(Clone)]
struct ActiveInstall {
    id: TaskId,
    key: InstallKey,
}

#[derive(Clone, PartialEq, Eq)]
struct InstallKey {
    root: PathBuf,
    version_id: String,
}

impl TaskManager {
    pub fn new(installer: VanillaInstaller, event_capacity: usize) -> Self {
        let (events, _) = broadcast::channel(event_capacity.max(1));
        Self {
            inner: Arc::new(TaskInner {
                installer,
                next_id: AtomicU64::new(1),
                registry: Mutex::new(Registry::default()),
                events,
            }),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<TaskEvent> {
        self.inner.events.subscribe()
    }

    pub async fn start_install(
        &self,
        mut request: VanillaInstallRequest,
    ) -> Result<TaskId, AppError> {
        request.minecraft_root =
            std::path::absolute(&request.minecraft_root).map_err(|source| AppError::Io {
                operation: "resolve Minecraft root",
                path: request.minecraft_root.clone(),
                source,
            })?;
        VersionId::new(request.version_id.clone())?;
        let key = InstallKey {
            root: request.minecraft_root.clone(),
            version_id: request.version_id.clone(),
        };
        let mut registry = self.inner.registry.lock().await;
        if let Some(existing) = registry
            .active_installs
            .iter()
            .find(|active| active.key == key)
        {
            return Err(AppError::TaskAlreadyRunning {
                existing: existing.id,
            });
        }
        let id = TaskId(self.inner.next_id.fetch_add(1, Ordering::Relaxed));
        let cancellation = CancellationToken::new();
        let snapshot = TaskSnapshot {
            id,
            kind: TaskKind::VanillaInstall,
            status: TaskStatus::Running,
            phase: None,
            progress: None,
            error: None,
        };
        registry.tasks.insert(
            id,
            TaskRecord {
                snapshot: snapshot.clone(),
                cancellation: cancellation.clone(),
            },
        );
        registry.active_installs.push(ActiveInstall { id, key });
        drop(registry);
        let _ = self.inner.events.send(TaskEvent::Snapshot(snapshot));

        let (sender, receiver) = mpsc::channel(128);
        let manager = self.clone();
        tokio::spawn(async move {
            manager
                .run_install(id, request, cancellation, sender, receiver)
                .await;
        });
        Ok(id)
    }

    pub async fn snapshot(&self, id: TaskId) -> Option<TaskSnapshot> {
        let registry = self.inner.registry.lock().await;
        registry
            .tasks
            .get(&id)
            .map(|record| record.snapshot.clone())
    }

    pub async fn cancel(&self, id: TaskId) -> bool {
        let (snapshot, cancellation) = {
            let mut registry = self.inner.registry.lock().await;
            let Some(record) = registry.tasks.get_mut(&id) else {
                return false;
            };
            if record.snapshot.status != TaskStatus::Running {
                return false;
            }
            record.snapshot.status = TaskStatus::Cancelling;
            (record.snapshot.clone(), record.cancellation.clone())
        };
        let _ = self.inner.events.send(TaskEvent::Snapshot(snapshot));
        cancellation.cancel();
        true
    }

    pub async fn dismiss(&self, id: TaskId) -> bool {
        let removed = {
            let mut registry = self.inner.registry.lock().await;
            let terminal = registry
                .tasks
                .get(&id)
                .is_some_and(|record| record.snapshot.status.is_terminal());
            if terminal {
                registry.tasks.remove(&id);
                registry.active_installs.retain(|active| active.id != id);
                true
            } else {
                false
            }
        };
        if removed {
            let _ = self.inner.events.send(TaskEvent::Dismissed(id));
        }
        removed
    }

    async fn run_install(
        self,
        id: TaskId,
        request: VanillaInstallRequest,
        cancellation: CancellationToken,
        sender: mpsc::Sender<InstallEvent>,
        mut receiver: mpsc::Receiver<InstallEvent>,
    ) {
        let installer = self.inner.installer.clone();
        let install = installer.install(request, cancellation, Some(sender));
        let forward = async {
            while let Some(event) = receiver.recv().await {
                self.apply_install_event(id, event).await;
            }
        };
        let (result, ()) = tokio::join!(install, forward);
        self.finish_install(id, result).await;
    }

    async fn apply_install_event(&self, id: TaskId, event: InstallEvent) {
        let snapshot = {
            let mut registry = self.inner.registry.lock().await;
            let Some(record) = registry.tasks.get_mut(&id) else {
                return;
            };
            match event {
                InstallEvent::Phase(phase) => record.snapshot.phase = Some(phase),
                InstallEvent::Download(progress) => {
                    record.snapshot.phase = Some(progress.phase);
                    record.snapshot.progress = Some(TaskProgress::Download(progress));
                }
                InstallEvent::Natives {
                    current,
                    completed_archives,
                    total_archives,
                } => {
                    record.snapshot.progress = Some(TaskProgress::Natives {
                        current,
                        completed_archives,
                        total_archives,
                    });
                }
            }
            record.snapshot.clone()
        };
        let _ = self.inner.events.send(TaskEvent::Snapshot(snapshot));
    }

    async fn finish_install(self, id: TaskId, result: Result<crate::InstallOutcome, AppError>) {
        let snapshot = {
            let mut registry = self.inner.registry.lock().await;
            registry.active_installs.retain(|active| active.id != id);
            let Some(record) = registry.tasks.get_mut(&id) else {
                return;
            };
            match result {
                Ok(_) => {
                    record.snapshot.status = TaskStatus::Completed;
                    record.snapshot.phase = Some(InstallPhase::Completed);
                    record.snapshot.error = None;
                }
                Err(AppError::Cancelled) => {
                    record.snapshot.status = TaskStatus::Cancelled;
                    record.snapshot.error = None;
                }
                Err(error) => {
                    record.snapshot.status = TaskStatus::Failed;
                    record.snapshot.error = Some(format_error(&error));
                }
            }
            record.snapshot.clone()
        };
        let _ = self.inner.events.send(TaskEvent::Snapshot(snapshot));
    }
}

fn format_error(error: &AppError) -> String {
    let mut message = error.to_string();
    if let AppError::DownloadBatch { failures, .. } = error {
        for failure in failures {
            message.push_str(&format!(
                "\n{}: {}",
                failure.destination.display(),
                failure.message
            ));
        }
    }
    message
}
