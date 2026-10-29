use std::path::PathBuf;

use rmcl_domain::ServerAddress;
use rmcl_java::{JavaRequirement, JavaRuntimeSource};
use rmcl_launch::{
    AuthlibLaunchConfig, GameEvent, GameExit, GameLog, GameProcess, LaunchAccount, LaunchError,
    LaunchOptions, LaunchPlan,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    AppError, InstallEvent, InstallOutcome, JavaEvent, JavaSelection, JavaSelectionOrigin,
    JavaService, VanillaInstallRequest, VanillaInstaller,
};

#[derive(Clone)]
pub struct GameLaunchRequest {
    pub install: VanillaInstallRequest,
    pub account: LaunchAccount,
    /// Explicit Java executable from the instance manifest. `None` uses
    /// automatic discovery and Mojang runtime installation.
    pub java_executable: Option<PathBuf>,
    /// Optional `-Xmx` value in MiB.
    pub max_memory_mb: Option<u32>,
    pub additional_jvm_arguments: Vec<String>,
    pub additional_game_arguments: Vec<String>,
    /// Optional authlib-injector agent for third-party Yggdrasil accounts.
    pub authlib: Option<AuthlibLaunchConfig>,
    /// Base Minecraft version used for version-dependent launch arguments.
    pub minecraft_version: String,
    /// Optional server to join immediately after launch.
    pub server: Option<ServerAddress>,
    pub resolution: Option<(u32, u32)>,
    pub demo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameLaunchEvent {
    Install(InstallEvent),
    Inspecting,
    Java(JavaEvent),
    Preparing,
    Starting,
    Started,
    Output(GameLog),
    Exited(GameExit),
}

pub type GameLaunchEventSender = mpsc::Sender<GameLaunchEvent>;

#[derive(Debug, Clone)]
pub struct GameLaunchOutcome {
    pub install: InstallOutcome,
    pub java: JavaSelection,
    pub exit: GameExit,
    pub stop_requested: bool,
}

#[derive(Clone)]
pub struct GameLauncher {
    installer: VanillaInstaller,
    java: JavaService,
}

impl GameLauncher {
    pub(crate) fn java_service(&self) -> JavaService {
        self.java.clone()
    }

    pub fn new(installer: VanillaInstaller, java: JavaService) -> Self {
        Self { installer, java }
    }

    /// Ensures the installation and Java runtime, then owns the game until it exits.
    pub async fn run(
        &self,
        request: GameLaunchRequest,
        cancellation: CancellationToken,
        events: Option<GameLaunchEventSender>,
    ) -> Result<GameLaunchOutcome, AppError> {
        let GameLaunchRequest {
            install,
            account,
            java_executable,
            max_memory_mb,
            additional_jvm_arguments,
            additional_game_arguments,
            authlib,
            minecraft_version,
            server,
            resolution,
            demo,
        } = request;
        let install = {
            let (sender, mut receiver) = mpsc::channel(128);
            // NOTE: Heap-pin sub-use-cases so this future is safe to nest in application tasks.
            let install = Box::pin(self.installer.install(
                install,
                cancellation.clone(),
                Some(sender),
            ));
            let forward = async {
                while let Some(event) = receiver.recv().await {
                    emit(&events, GameLaunchEvent::Install(event)).await;
                }
            };
            let (outcome, ()) = tokio::join!(install, forward);
            outcome?
        };
        self.finish_launch(
            install,
            account,
            java_executable,
            max_memory_mb,
            additional_jvm_arguments,
            additional_game_arguments,
            authlib,
            minecraft_version,
            server,
            resolution,
            demo,
            cancellation,
            events,
        )
        .await
    }

    /// Launches an already installed version without re-planning downloads.
    ///
    /// Base files and the natives directory are checked for existence, but
    /// asset objects are not re-hashed. Use [`Self::run`] when the caller needs
    /// the full repair pipeline.
    pub async fn run_installed(
        &self,
        request: GameLaunchRequest,
        cancellation: CancellationToken,
        events: Option<GameLaunchEventSender>,
    ) -> Result<GameLaunchOutcome, AppError> {
        let GameLaunchRequest {
            install,
            account,
            java_executable,
            max_memory_mb,
            additional_jvm_arguments,
            additional_game_arguments,
            authlib,
            minecraft_version,
            server,
            resolution,
            demo,
        } = request;
        emit(&events, GameLaunchEvent::Inspecting).await;
        let install = self
            .installer
            .inspect_existing(&install, cancellation.clone())
            .await?;
        self.finish_launch(
            install,
            account,
            java_executable,
            max_memory_mb,
            additional_jvm_arguments,
            additional_game_arguments,
            authlib,
            minecraft_version,
            server,
            resolution,
            demo,
            cancellation,
            events,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn finish_launch(
        &self,
        install: InstallOutcome,
        account: LaunchAccount,
        java_executable: Option<PathBuf>,
        max_memory_mb: Option<u32>,
        additional_jvm_arguments: Vec<String>,
        additional_game_arguments: Vec<String>,
        authlib: Option<AuthlibLaunchConfig>,
        minecraft_version: String,
        server: Option<ServerAddress>,
        resolution: Option<(u32, u32)>,
        demo: bool,
        cancellation: CancellationToken,
        events: Option<GameLaunchEventSender>,
    ) -> Result<GameLaunchOutcome, AppError> {
        let requirement = JavaRequirement::for_version(&install.version, &install.platform);
        let java = if let Some(executable) = java_executable {
            emit(&events, GameLaunchEvent::Java(JavaEvent::Checking)).await;
            check_cancelled(&cancellation)?;
            let runtime = rmcl_java::probe(executable.clone(), JavaRuntimeSource::Custom).await?;
            check_cancelled(&cancellation)?;
            if !requirement.accepts(&runtime) {
                return Err(AppError::JavaOverrideIncompatible {
                    path: executable,
                    version: runtime.version,
                    requirement: Box::new(requirement),
                });
            }
            JavaSelection {
                runtime,
                origin: JavaSelectionOrigin::Custom,
            }
        } else {
            let (sender, mut receiver) = mpsc::channel(8);
            let ensure = Box::pin(self.java.ensure(
                &requirement,
                cancellation.clone(),
                Some(sender),
            ));
            let forward = async {
                while let Some(event) = receiver.recv().await {
                    emit(&events, GameLaunchEvent::Java(event)).await;
                }
            };
            let (selection, ()) = tokio::join!(ensure, forward);
            selection?
        };

        emit(&events, GameLaunchEvent::Preparing).await;
        let mut options = LaunchOptions::from_install_plan(
            &install.instance,
            java.runtime.clone(),
            &install.version,
            account,
            &install.platform,
            &install.base_plan,
        )?;
        options.additional_jvm_arguments = additional_jvm_arguments;
        if let Some(memory) = max_memory_mb {
            options
                .additional_jvm_arguments
                .push(format!("-Xmx{memory}M"));
        }
        options.additional_game_arguments = additional_game_arguments;
        options.authlib = authlib;
        if let Some(server) = server {
            options = options.with_server(server, minecraft_version);
        } else {
            options.minecraft_version = minecraft_version;
        }
        if let Some((width, height)) = resolution {
            options = options.with_resolution(width, height);
        }
        options = options.with_demo(demo);
        let plan = LaunchPlan::from_options(&install.version, options)?;
        emit(&events, GameLaunchEvent::Starting).await;
        check_cancelled(&cancellation)?;
        let mut process = GameProcess::spawn(&plan)?;
        emit(&events, GameLaunchEvent::Started).await;

        let mut stop_requested = false;
        let exit = loop {
            let event = tokio::select! {
                event = process.next_event() => event,
                _ = cancellation.cancelled(), if !stop_requested => {
                    stop_requested = true;
                    process.kill().await?;
                    continue;
                }
            };
            match event? {
                Some(GameEvent::Output(log)) => {
                    emit(&events, GameLaunchEvent::Output(log)).await;
                }
                Some(GameEvent::Exit(exit)) => break exit,
                None => {
                    return Err(LaunchError::ProcessTask(
                        "game event stream ended before an exit event".to_owned(),
                    )
                    .into());
                }
            }
        };
        emit(&events, GameLaunchEvent::Exited(exit.clone())).await;
        Ok(GameLaunchOutcome {
            install,
            java,
            exit,
            stop_requested,
        })
    }
}

fn check_cancelled(cancellation: &CancellationToken) -> Result<(), AppError> {
    if cancellation.is_cancelled() {
        Err(AppError::Cancelled)
    } else {
        Ok(())
    }
}

async fn emit(events: &Option<GameLaunchEventSender>, event: GameLaunchEvent) {
    if let Some(events) = events {
        let _ = events.send(event).await;
    }
}
