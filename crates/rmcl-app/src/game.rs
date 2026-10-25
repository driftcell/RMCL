use rmcl_auth::OfflineAccount;
use rmcl_java::JavaRequirement;
use rmcl_launch::{
    GameEvent, GameExit, GameLog, GameProcess, LaunchAccount, LaunchError, LaunchOptions,
    LaunchPlan,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    AppError, InstallEvent, InstallOutcome, JavaEvent, JavaSelection, JavaService,
    VanillaInstallRequest, VanillaInstaller,
};

#[derive(Clone)]
pub struct GameLaunchRequest {
    pub install: VanillaInstallRequest,
    pub account: OfflineAccount,
    pub additional_jvm_arguments: Vec<String>,
    pub additional_game_arguments: Vec<String>,
    pub resolution: Option<(u32, u32)>,
    pub demo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameLaunchEvent {
    Install(InstallEvent),
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
            additional_jvm_arguments,
            additional_game_arguments,
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
        let requirement = JavaRequirement::for_version(&install.version, &install.platform);
        let java = {
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
        let account = LaunchAccount::from_offline(&account);
        let mut options = LaunchOptions::from_install_plan(
            &install.layout,
            java.runtime.clone(),
            &install.version,
            account,
            &install.platform,
            &install.base_plan,
        )?;
        options.additional_jvm_arguments = additional_jvm_arguments;
        options.additional_game_arguments = additional_game_arguments;
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
