use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use rmcl_instance::VersionId;
use rmcl_launch::{GameExit, GameLog};
use tokio::sync::{Mutex, broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::{
    AppError, GameLaunchEvent, GameLaunchOutcome, GameLaunchRequest, GameLauncher,
    InstallDownloadProgress, InstallEvent, InstallOutcome, InstallPhase, JavaEvent,
    VanillaInstallRequest, VanillaInstaller,
};

const GAME_LOG_CAPACITY: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskId(u64);

impl TaskId {
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
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
    GameLaunch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskPhase {
    Install(InstallPhase),
    CheckingJava,
    InstallingJava,
    PreparingLaunch,
    StartingGame,
    RunningGame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Running,
    Cancelling,
    Completed,
    Cancelled,
    Failed,
    Exited,
}

impl TaskStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Cancelled | Self::Failed | Self::Exited
        )
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
    pub phase: Option<TaskPhase>,
    pub progress: Option<TaskProgress>,
    pub error: Option<String>,
    pub exit: Option<GameExit>,
    pub stop_requested: bool,
    pub log_lines: u64,
}

#[derive(Debug, Clone)]
pub enum TaskEvent {
    Snapshot(TaskSnapshot),
    GameLog {
        task_id: TaskId,
        line: u64,
        log: GameLog,
    },
    Dismissed(TaskId),
}

#[derive(Clone)]
pub struct TaskManager {
    inner: Arc<TaskInner>,
}

struct TaskInner {
    installer: VanillaInstaller,
    game: Option<GameLauncher>,
    next_id: AtomicU64,
    registry: Mutex<Registry>,
    events: broadcast::Sender<TaskEvent>,
}

#[derive(Default)]
struct Registry {
    tasks: TaskMap,
    active_versions: Vec<ActiveVersion>,
}

type TaskMap = std::collections::HashMap<TaskId, TaskRecord>;

struct TaskRecord {
    snapshot: TaskSnapshot,
    cancellation: CancellationToken,
    logs: VecDeque<GameLog>,
}

#[derive(Clone)]
struct ActiveVersion {
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
        Self::build(installer, None, event_capacity)
    }

    pub fn with_java(
        installer: VanillaInstaller,
        java: crate::JavaService,
        event_capacity: usize,
    ) -> Self {
        let game = GameLauncher::new(installer.clone(), java);
        Self::build(installer, Some(game), event_capacity)
    }

    fn build(
        installer: VanillaInstaller,
        game: Option<GameLauncher>,
        event_capacity: usize,
    ) -> Self {
        let (events, _) = broadcast::channel(event_capacity.max(1));
        Self {
            inner: Arc::new(TaskInner {
                installer,
                game,
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
        let key = prepare_request(&mut request)?;
        let (id, cancellation) = self.register(TaskKind::VanillaInstall, key).await?;
        let (sender, receiver) = mpsc::channel(128);
        let manager = self.clone();
        tokio::spawn(async move {
            manager
                .run_install(id, request, cancellation, sender, receiver)
                .await;
        });
        Ok(id)
    }

    pub async fn start_launch(&self, mut request: GameLaunchRequest) -> Result<TaskId, AppError> {
        let Some(game) = self.inner.game.clone() else {
            return Err(AppError::JavaNotConfigured);
        };
        let key = prepare_request(&mut request.install)?;
        let (id, cancellation) = self.register(TaskKind::GameLaunch, key).await?;
        let (sender, receiver) = mpsc::channel(128);
        let manager = self.clone();
        tokio::spawn(async move {
            manager
                .run_launch(id, game, request, cancellation, sender, receiver)
                .await;
        });
        Ok(id)
    }

    async fn register(
        &self,
        kind: TaskKind,
        key: InstallKey,
    ) -> Result<(TaskId, CancellationToken), AppError> {
        let mut registry = self.inner.registry.lock().await;
        if let Some(existing) = registry
            .active_versions
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
            kind,
            status: TaskStatus::Running,
            phase: None,
            progress: None,
            error: None,
            exit: None,
            stop_requested: false,
            log_lines: 0,
        };
        registry.tasks.insert(
            id,
            TaskRecord {
                snapshot: snapshot.clone(),
                cancellation: cancellation.clone(),
                logs: VecDeque::new(),
            },
        );
        registry.active_versions.push(ActiveVersion { id, key });
        drop(registry);
        let _ = self.inner.events.send(TaskEvent::Snapshot(snapshot));
        Ok((id, cancellation))
    }

    pub async fn snapshot(&self, id: TaskId) -> Option<TaskSnapshot> {
        let registry = self.inner.registry.lock().await;
        registry
            .tasks
            .get(&id)
            .map(|record| record.snapshot.clone())
    }

    pub async fn logs(&self, id: TaskId) -> Option<Vec<GameLog>> {
        let registry = self.inner.registry.lock().await;
        registry
            .tasks
            .get(&id)
            .map(|record| record.logs.iter().cloned().collect())
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
                registry.active_versions.retain(|active| active.id != id);
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
        // NOTE: Heap-pin the large orchestration future instead of polling it on a worker stack.
        let install = Box::pin(installer.install(request, cancellation, Some(sender)));
        let forward = async {
            while let Some(event) = receiver.recv().await {
                self.apply_install_event(id, event).await;
            }
        };
        let (result, ()) = tokio::join!(install, forward);
        self.finish_install(id, result).await;
    }

    async fn run_launch(
        self,
        id: TaskId,
        game: GameLauncher,
        request: GameLaunchRequest,
        cancellation: CancellationToken,
        sender: mpsc::Sender<GameLaunchEvent>,
        mut receiver: mpsc::Receiver<GameLaunchEvent>,
    ) {
        // NOTE: Keep the larger install-to-exit future off the worker stack as well.
        let run = Box::pin(game.run(request, cancellation, Some(sender)));
        let forward = async {
            while let Some(event) = receiver.recv().await {
                self.apply_game_event(id, event).await;
            }
        };
        let (result, ()) = tokio::join!(run, forward);
        self.finish_launch(id, result).await;
    }

    async fn apply_install_event(&self, id: TaskId, event: InstallEvent) {
        let snapshot = {
            let mut registry = self.inner.registry.lock().await;
            let Some(record) = registry.tasks.get_mut(&id) else {
                return;
            };
            match event {
                InstallEvent::Phase(phase) => {
                    record.snapshot.phase = Some(TaskPhase::Install(phase));
                }
                InstallEvent::Download(progress) => {
                    record.snapshot.phase = Some(TaskPhase::Install(progress.phase));
                    record.snapshot.progress = Some(TaskProgress::Download(progress));
                }
                InstallEvent::Natives {
                    current,
                    completed_archives,
                    total_archives,
                } => {
                    record.snapshot.phase = Some(TaskPhase::Install(InstallPhase::MergingNatives));
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

    async fn apply_game_event(&self, id: TaskId, event: GameLaunchEvent) {
        match event {
            GameLaunchEvent::Install(event) => self.apply_install_event(id, event).await,
            GameLaunchEvent::Java(JavaEvent::Checking) => {
                self.update_phase(id, TaskPhase::CheckingJava).await;
            }
            GameLaunchEvent::Java(JavaEvent::Installing) => {
                self.update_phase(id, TaskPhase::InstallingJava).await;
            }
            GameLaunchEvent::Preparing => {
                self.update_phase(id, TaskPhase::PreparingLaunch).await;
            }
            GameLaunchEvent::Starting => {
                self.update_phase(id, TaskPhase::StartingGame).await;
            }
            GameLaunchEvent::Started => {
                self.update_phase(id, TaskPhase::RunningGame).await;
            }
            GameLaunchEvent::Output(log) => self.append_log(id, log).await,
            GameLaunchEvent::Exited(exit) => {
                let mut registry = self.inner.registry.lock().await;
                if let Some(record) = registry.tasks.get_mut(&id) {
                    record.snapshot.exit = Some(exit);
                }
            }
        }
    }

    async fn update_phase(&self, id: TaskId, phase: TaskPhase) {
        let snapshot = {
            let mut registry = self.inner.registry.lock().await;
            let Some(record) = registry.tasks.get_mut(&id) else {
                return;
            };
            record.snapshot.phase = Some(phase);
            record.snapshot.clone()
        };
        let _ = self.inner.events.send(TaskEvent::Snapshot(snapshot));
    }

    async fn append_log(&self, id: TaskId, log: GameLog) {
        let line = {
            let mut registry = self.inner.registry.lock().await;
            let Some(record) = registry.tasks.get_mut(&id) else {
                return;
            };
            if record.logs.len() == GAME_LOG_CAPACITY {
                record.logs.pop_front();
            }
            record.logs.push_back(log.clone());
            record.snapshot.log_lines += 1;
            record.snapshot.log_lines
        };
        let _ = self.inner.events.send(TaskEvent::GameLog {
            task_id: id,
            line,
            log,
        });
    }

    async fn finish_install(self, id: TaskId, result: Result<InstallOutcome, AppError>) {
        let snapshot = {
            let mut registry = self.inner.registry.lock().await;
            registry.active_versions.retain(|active| active.id != id);
            let Some(record) = registry.tasks.get_mut(&id) else {
                return;
            };
            match result {
                Ok(_) => {
                    record.snapshot.status = TaskStatus::Completed;
                    record.snapshot.phase = Some(TaskPhase::Install(InstallPhase::Completed));
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

    async fn finish_launch(self, id: TaskId, result: Result<GameLaunchOutcome, AppError>) {
        let snapshot = {
            let mut registry = self.inner.registry.lock().await;
            registry.active_versions.retain(|active| active.id != id);
            let Some(record) = registry.tasks.get_mut(&id) else {
                return;
            };
            match result {
                Ok(outcome) => {
                    record.snapshot.status = TaskStatus::Exited;
                    record.snapshot.exit = Some(outcome.exit);
                    record.snapshot.stop_requested = outcome.stop_requested;
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

fn prepare_request(request: &mut VanillaInstallRequest) -> Result<InstallKey, AppError> {
    request.minecraft_root =
        std::path::absolute(&request.minecraft_root).map_err(|source| AppError::Io {
            operation: "resolve Minecraft root",
            path: request.minecraft_root.clone(),
            source,
        })?;
    VersionId::new(request.version_id.clone())?;
    Ok(InstallKey {
        root: request.minecraft_root.clone(),
        version_id: request.version_id.clone(),
    })
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
