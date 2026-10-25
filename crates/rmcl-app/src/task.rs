use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use rmcl_content::ContentKind;
use rmcl_instance::VersionId;
use rmcl_launch::{GameExit, GameLog};
use tokio::sync::{Mutex, broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::{
    AppError, ContentDownloadProgress, ContentGraphInstallRequest, ContentInstallEvent,
    ContentInstallPhase, ContentInstallRequest, ContentInstaller, CreateInstanceRequest,
    GameLaunchEvent, GameLaunchOutcome, GameLaunchRequest, GameLauncher, InstallDownloadProgress,
    InstallEvent, InstallOutcome, InstallPhase, InstanceCreateEvent, InstanceCreateOutcome,
    InstanceCreatePhase, InstanceCreator, JavaEvent, ModrinthClient, VanillaInstallRequest,
    VanillaInstaller, content::absolute_instance, content::resolve_content_install_batch,
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
    InstanceCreate,
    ContentInstall,
    GameLaunch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskPhase {
    Install(InstallPhase),
    InstanceCreate(InstanceCreatePhase),
    Content(ContentInstallPhase),
    Inspecting,
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
    Content(ContentDownloadProgress),
    Processors {
        current: String,
        completed: usize,
        total: usize,
    },
    Natives {
        current: PathBuf,
        completed_archives: usize,
        total_archives: usize,
    },
}

/// Stable task subject exposed to the UI without parsing snapshot errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskSubject {
    Version {
        id: String,
    },
    Content {
        instance_id: String,
        kind: ContentKind,
        name: String,
    },
    Instance {
        id: String,
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSnapshot {
    pub id: TaskId,
    pub kind: TaskKind,
    pub subject: Option<TaskSubject>,
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
    content: Option<ContentInstaller>,
    modrinth: Option<ModrinthClient>,
    instance_creator: Option<InstanceCreator>,
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
        Self::build(installer, None, None, None, None, event_capacity)
    }

    pub fn with_java(
        installer: VanillaInstaller,
        java: crate::JavaService,
        event_capacity: usize,
    ) -> Self {
        let game = GameLauncher::new(installer.clone(), java);
        Self::build(installer, Some(game), None, None, None, event_capacity)
    }

    pub fn with_content(
        installer: VanillaInstaller,
        java: crate::JavaService,
        content: ContentInstaller,
        event_capacity: usize,
    ) -> Self {
        let game = GameLauncher::new(installer.clone(), java);
        Self::build(
            installer,
            Some(game),
            Some(content),
            None,
            None,
            event_capacity,
        )
    }

    pub fn with_full(
        installer: VanillaInstaller,
        java: crate::JavaService,
        content: ContentInstaller,
        instance_creator: InstanceCreator,
        event_capacity: usize,
    ) -> Self {
        let game = GameLauncher::new(installer.clone(), java);
        Self::build(
            installer,
            Some(game),
            Some(content),
            Some(instance_creator),
            None,
            event_capacity,
        )
    }

    pub fn with_full_and_modrinth(
        installer: VanillaInstaller,
        java: crate::JavaService,
        content: ContentInstaller,
        instance_creator: InstanceCreator,
        modrinth: ModrinthClient,
        event_capacity: usize,
    ) -> Self {
        let game = GameLauncher::new(installer.clone(), java);
        Self::build(
            installer,
            Some(game),
            Some(content),
            Some(instance_creator),
            Some(modrinth),
            event_capacity,
        )
    }

    /// Enables content installs without requiring a Java/game launcher.
    pub fn with_content_only(
        installer: VanillaInstaller,
        content: ContentInstaller,
        event_capacity: usize,
    ) -> Self {
        Self::build(installer, None, Some(content), None, None, event_capacity)
    }

    /// Enables remote content graphs without requiring a Java/game launcher.
    pub fn with_content_and_modrinth(
        installer: VanillaInstaller,
        content: ContentInstaller,
        modrinth: ModrinthClient,
        event_capacity: usize,
    ) -> Self {
        Self::build(
            installer,
            None,
            Some(content),
            None,
            Some(modrinth),
            event_capacity,
        )
    }

    /// Enables instance creation without requiring a Java/game launcher.
    pub fn with_creator_only(
        installer: VanillaInstaller,
        instance_creator: InstanceCreator,
        event_capacity: usize,
    ) -> Self {
        Self::build(
            installer,
            None,
            None,
            Some(instance_creator),
            None,
            event_capacity,
        )
    }

    fn build(
        installer: VanillaInstaller,
        game: Option<GameLauncher>,
        content: Option<ContentInstaller>,
        instance_creator: Option<InstanceCreator>,
        modrinth: Option<ModrinthClient>,
        event_capacity: usize,
    ) -> Self {
        let (events, _) = broadcast::channel(event_capacity.max(1));
        Self {
            inner: Arc::new(TaskInner {
                installer,
                game,
                content,
                modrinth,
                instance_creator,
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
        let subject = TaskSubject::Version {
            id: key.version_id.clone(),
        };
        let (id, cancellation) = self
            .register(TaskKind::VanillaInstall, key, Some(subject))
            .await?;
        let (sender, receiver) = mpsc::channel(128);
        let manager = self.clone();
        tokio::spawn(async move {
            manager
                .run_install(id, request, cancellation, sender, receiver)
                .await;
        });
        Ok(id)
    }

    /// Creates an isolated Vanilla or Fabric/Quilt instance.
    pub async fn start_create_instance(
        &self,
        mut request: CreateInstanceRequest,
    ) -> Result<TaskId, AppError> {
        let Some(creator) = self.inner.instance_creator.clone() else {
            return Err(AppError::InstanceCreatorNotConfigured);
        };
        let key = prepare_create_request(&mut request)?;
        let subject = TaskSubject::Instance {
            id: key.version_id.clone(),
            name: if request.name.trim().is_empty() {
                key.version_id.clone()
            } else {
                request.name.trim().to_owned()
            },
        };
        let (id, cancellation) = self
            .register(TaskKind::InstanceCreate, key, Some(subject))
            .await?;
        let (sender, receiver) = mpsc::channel(128);
        let manager = self.clone();
        tokio::spawn(async move {
            manager
                .run_create_instance(id, creator, request, cancellation, sender, receiver)
                .await;
        });
        Ok(id)
    }

    /// Downloads and installs one remote content file into an instance.
    pub async fn start_content_install(
        &self,
        mut request: ContentInstallRequest,
    ) -> Result<TaskId, AppError> {
        let Some(content) = self.inner.content.clone() else {
            return Err(AppError::RemoteContentNotConfigured);
        };
        let key = prepare_content_request(&mut request)?;
        let subject = TaskSubject::Content {
            instance_id: key.version_id.clone(),
            kind: request.kind,
            name: request.display_name.clone(),
        };
        let (id, cancellation) = self
            .register(TaskKind::ContentInstall, key, Some(subject))
            .await?;
        let (sender, receiver) = mpsc::channel(128);
        let manager = self.clone();
        tokio::spawn(async move {
            manager
                .run_content_install(id, content, request, cancellation, sender, receiver)
                .await;
        });
        Ok(id)
    }

    /// Installs a Modrinth version together with its required dependencies.
    ///
    /// The graph is resolved inside the task so resolution failures share the
    /// same task terminal state, cancellation and subject as the downloads.
    pub async fn start_content_install_with_dependencies(
        &self,
        mut request: ContentGraphInstallRequest,
    ) -> Result<TaskId, AppError> {
        let Some(content) = self.inner.content.clone() else {
            return Err(AppError::RemoteContentNotConfigured);
        };
        let Some(modrinth) = self.inner.modrinth.clone() else {
            return Err(AppError::RemoteContentNotConfigured);
        };
        let key = prepare_content_graph_request(&mut request)?;
        let subject = TaskSubject::Content {
            instance_id: key.version_id.clone(),
            kind: request.kind,
            name: request.display_name.clone(),
        };
        let (id, cancellation) = self
            .register(TaskKind::ContentInstall, key, Some(subject))
            .await?;
        let (sender, receiver) = mpsc::channel(128);
        let manager = self.clone();
        tokio::spawn(async move {
            manager
                .run_content_graph(
                    id,
                    content,
                    modrinth,
                    request,
                    cancellation,
                    sender,
                    receiver,
                )
                .await;
        });
        Ok(id)
    }

    pub async fn start_launch(&self, request: GameLaunchRequest) -> Result<TaskId, AppError> {
        self.start_launch_with(request, false).await
    }

    /// Launches an installed version without the ensure/repair pass.
    ///
    /// The caller must provide a version whose base files are present. Missing
    /// files fail the task with a structured incomplete-installation error.
    pub async fn start_launch_installed(
        &self,
        request: GameLaunchRequest,
    ) -> Result<TaskId, AppError> {
        self.start_launch_with(request, true).await
    }

    async fn start_launch_with(
        &self,
        mut request: GameLaunchRequest,
        installed: bool,
    ) -> Result<TaskId, AppError> {
        let Some(game) = self.inner.game.clone() else {
            return Err(AppError::JavaNotConfigured);
        };
        let key = prepare_request(&mut request.install)?;
        let subject = TaskSubject::Version {
            id: key.version_id.clone(),
        };
        let (id, cancellation) = self
            .register(TaskKind::GameLaunch, key, Some(subject))
            .await?;
        let (sender, receiver) = mpsc::channel(128);
        if installed {
            self.update_phase(id, TaskPhase::Inspecting).await;
        }
        let manager = self.clone();
        tokio::spawn(async move {
            manager
                .run_launch(id, game, request, cancellation, sender, receiver, installed)
                .await;
        });
        Ok(id)
    }

    async fn register(
        &self,
        kind: TaskKind,
        key: InstallKey,
        subject: Option<TaskSubject>,
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
            subject,
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

    async fn run_create_instance(
        self,
        id: TaskId,
        creator: InstanceCreator,
        request: CreateInstanceRequest,
        cancellation: CancellationToken,
        sender: mpsc::Sender<InstanceCreateEvent>,
        mut receiver: mpsc::Receiver<InstanceCreateEvent>,
    ) {
        let create = Box::pin(creator.create(request, cancellation, Some(sender)));
        let forward = async {
            while let Some(event) = receiver.recv().await {
                self.apply_instance_create_event(id, event).await;
            }
        };
        let (result, ()) = tokio::join!(create, forward);
        self.finish_create_instance(id, result).await;
    }

    async fn run_content_install(
        self,
        id: TaskId,
        installer: ContentInstaller,
        request: ContentInstallRequest,
        cancellation: CancellationToken,
        sender: mpsc::Sender<ContentInstallEvent>,
        mut receiver: mpsc::Receiver<ContentInstallEvent>,
    ) {
        let install = Box::pin(installer.install(request, cancellation, Some(sender)));
        let forward = async {
            while let Some(event) = receiver.recv().await {
                self.apply_content_event(id, event).await;
            }
        };
        let (result, ()) = tokio::join!(install, forward);
        self.finish_content(id, result.map(|_| ())).await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_content_graph(
        self,
        id: TaskId,
        installer: ContentInstaller,
        modrinth: ModrinthClient,
        request: ContentGraphInstallRequest,
        cancellation: CancellationToken,
        sender: mpsc::Sender<ContentInstallEvent>,
        mut receiver: mpsc::Receiver<ContentInstallEvent>,
    ) {
        self.update_phase(id, TaskPhase::Content(ContentInstallPhase::Preparing))
            .await;
        let result = async {
            let requests =
                resolve_content_install_batch(&modrinth, &request, &cancellation).await?;
            if requests.is_empty() {
                return Ok(());
            }
            let install =
                Box::pin(installer.install_batch(requests, cancellation.clone(), Some(sender)));
            let forward = async {
                while let Some(event) = receiver.recv().await {
                    self.apply_content_event(id, event).await;
                }
            };
            let (result, ()) = tokio::join!(install, forward);
            result.map(|_| ())
        }
        .await;
        self.finish_content(id, result).await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_launch(
        self,
        id: TaskId,
        game: GameLauncher,
        request: GameLaunchRequest,
        cancellation: CancellationToken,
        sender: mpsc::Sender<GameLaunchEvent>,
        mut receiver: mpsc::Receiver<GameLaunchEvent>,
        installed: bool,
    ) {
        // NOTE: Keep the larger install-to-exit future off the worker stack as well.
        let run = Box::pin(async move {
            if installed {
                game.run_installed(request, cancellation, Some(sender))
                    .await
            } else {
                game.run(request, cancellation, Some(sender)).await
            }
        });
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

    async fn apply_instance_create_event(&self, id: TaskId, event: InstanceCreateEvent) {
        match event {
            InstanceCreateEvent::Phase(phase) => {
                self.update_phase(id, TaskPhase::InstanceCreate(phase))
                    .await;
            }
            InstanceCreateEvent::Install(event) => self.apply_install_event(id, event).await,
            InstanceCreateEvent::Processor {
                current,
                completed,
                total,
            } => {
                let snapshot = {
                    let mut registry = self.inner.registry.lock().await;
                    let Some(record) = registry.tasks.get_mut(&id) else {
                        return;
                    };
                    record.snapshot.phase = Some(TaskPhase::InstanceCreate(
                        InstanceCreatePhase::InstallingLoader,
                    ));
                    record.snapshot.progress = Some(TaskProgress::Processors {
                        current,
                        completed,
                        total,
                    });
                    record.snapshot.clone()
                };
                let _ = self.inner.events.send(TaskEvent::Snapshot(snapshot));
            }
        }
    }

    async fn apply_content_event(&self, id: TaskId, event: ContentInstallEvent) {
        let snapshot = {
            let mut registry = self.inner.registry.lock().await;
            let Some(record) = registry.tasks.get_mut(&id) else {
                return;
            };
            match event {
                ContentInstallEvent::Phase(phase) => {
                    record.snapshot.phase = Some(TaskPhase::Content(phase));
                }
                ContentInstallEvent::Download(progress) => {
                    record.snapshot.phase = Some(TaskPhase::Content(progress.phase));
                    record.snapshot.progress = Some(TaskProgress::Content(progress));
                }
            }
            record.snapshot.clone()
        };
        let _ = self.inner.events.send(TaskEvent::Snapshot(snapshot));
    }

    async fn apply_game_event(&self, id: TaskId, event: GameLaunchEvent) {
        match event {
            GameLaunchEvent::Install(event) => self.apply_install_event(id, event).await,
            GameLaunchEvent::Inspecting => {
                self.update_phase(id, TaskPhase::Inspecting).await;
            }
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

    async fn finish_create_instance(
        self,
        id: TaskId,
        result: Result<InstanceCreateOutcome, AppError>,
    ) {
        let snapshot = {
            let mut registry = self.inner.registry.lock().await;
            registry.active_versions.retain(|active| active.id != id);
            let Some(record) = registry.tasks.get_mut(&id) else {
                return;
            };
            match result {
                Ok(_) => {
                    record.snapshot.status = TaskStatus::Completed;
                    record.snapshot.phase =
                        Some(TaskPhase::InstanceCreate(InstanceCreatePhase::Completed));
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

    async fn finish_content(self, id: TaskId, result: Result<(), AppError>) {
        let snapshot = {
            let mut registry = self.inner.registry.lock().await;
            registry.active_versions.retain(|active| active.id != id);
            let Some(record) = registry.tasks.get_mut(&id) else {
                return;
            };
            match result {
                Ok(_) => {
                    record.snapshot.status = TaskStatus::Completed;
                    record.snapshot.phase =
                        Some(TaskPhase::Content(ContentInstallPhase::Completed));
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

fn prepare_create_request(request: &mut CreateInstanceRequest) -> Result<InstallKey, AppError> {
    request.minecraft_root =
        std::path::absolute(&request.minecraft_root).map_err(|source| AppError::Io {
            operation: "resolve Minecraft root",
            path: request.minecraft_root.clone(),
            source,
        })?;
    VersionId::new(request.instance_id.clone())?;
    Ok(InstallKey {
        root: request.minecraft_root.clone(),
        version_id: request.instance_id.clone(),
    })
}

fn prepare_content_request(request: &mut ContentInstallRequest) -> Result<InstallKey, AppError> {
    request.instance = absolute_instance(&request.instance)?;
    Ok(InstallKey {
        root: request.instance.repository().root().to_path_buf(),
        version_id: request.instance.id().as_str().to_owned(),
    })
}

fn prepare_content_graph_request(
    request: &mut ContentGraphInstallRequest,
) -> Result<InstallKey, AppError> {
    request.instance = absolute_instance(&request.instance)?;
    Ok(InstallKey {
        root: request.instance.repository().root().to_path_buf(),
        version_id: request.instance.id().as_str().to_owned(),
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
