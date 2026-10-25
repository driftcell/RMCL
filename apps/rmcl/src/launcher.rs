use std::rc::Rc;

use gpui_kit::{
    base::Disableable,
    component::{
        ActiveTheme,
        button::{Button, ButtonVariants},
        form::{Field, Form},
        input::{Input, InputState},
        progress::Progress,
    },
    prelude::FluentBuilder,
    *,
};
use rmcl_app::{
    GameLaunchRequest, InstallPhase, TaskEvent, TaskPhase, TaskProgress, TaskSnapshot, TaskStatus,
    VanillaInstallRequest, VersionSource,
};
use rmcl_auth::OfflineAccount;
use rmcl_launch::LogStream;
use tokio::sync::broadcast::error::RecvError;

use crate::{
    services::{Bootstrap, Services},
    state::LauncherState,
};

pub struct LauncherView {
    services: Rc<Services>,
    minecraft_root: Entity<InputState>,
    version_id: Entity<InputState>,
    username: Entity<InputState>,
    state: LauncherState,
    log_scroll: ScrollHandle,
    _event_task: Task<()>,
    _command_task: Option<Task<()>>,
}

enum StartCommand {
    Install(VanillaInstallRequest),
    Launch(GameLaunchRequest),
}

impl LauncherView {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        services: Rc<Services>,
        bootstrap: Bootstrap,
    ) -> Self {
        let minecraft_root = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(bootstrap.minecraft_root.to_string_lossy().into_owned())
        });
        let version_id =
            cx.new(|cx| InputState::new(window, cx).default_value(bootstrap.version_id));
        let username = cx.new(|cx| InputState::new(window, cx).default_value(bootstrap.username));
        let _event_task = Self::spawn_event_bridge(&services, cx);
        Self {
            services,
            minecraft_root,
            version_id,
            username,
            state: LauncherState::new(),
            log_scroll: ScrollHandle::new(),
            _event_task,
            _command_task: None,
        }
    }

    fn spawn_event_bridge(services: &Rc<Services>, cx: &mut Context<Self>) -> Task<()> {
        let manager = services.tasks.clone();
        let mut events = manager.subscribe();
        cx.spawn(async move |this, cx| {
            loop {
                match events.recv().await {
                    Ok(TaskEvent::Snapshot(snapshot)) => {
                        if this
                            .update(cx, |view, cx| {
                                if view.state.apply_snapshot(snapshot) {
                                    cx.notify();
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(TaskEvent::GameLog { task_id, line, log }) => {
                        if this
                            .update(cx, |view, cx| {
                                if view.state.apply_log(task_id, line, log) {
                                    view.log_scroll.scroll_to_bottom();
                                    cx.notify();
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(TaskEvent::Dismissed(_)) => {}
                    Err(RecvError::Lagged(_)) => {
                        let Some(id) = this
                            .update(cx, |view, _| view.state.active_id())
                            .ok()
                            .flatten()
                        else {
                            continue;
                        };
                        let snapshot = manager.snapshot(id).await;
                        let logs = manager.logs(id).await;
                        if let Some(snapshot) = snapshot
                            && this
                                .update(cx, |view, cx| {
                                    view.state.recover(snapshot, logs.unwrap_or_default());
                                    view.log_scroll.scroll_to_bottom();
                                    cx.notify();
                                })
                                .is_err()
                        {
                            break;
                        }
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        })
    }

    fn install_request(&self, cx: &Context<Self>) -> Result<VanillaInstallRequest, String> {
        let root = self.minecraft_root.read(cx).value().to_string();
        let version_id = self.version_id.read(cx).value().trim().to_owned();
        if root.trim().is_empty() {
            return Err("Minecraft 目录不能为空".to_owned());
        }
        if version_id.is_empty() {
            return Err("版本 ID 不能为空".to_owned());
        }
        Ok(VanillaInstallRequest {
            minecraft_root: root.into(),
            version_id,
            platform: self.services.platform.clone(),
            source: VersionSource::Manifest,
        })
    }

    fn start_install(&mut self, cx: &mut Context<Self>) {
        if !self.state.can_start() {
            return;
        }
        match self.install_request(cx) {
            Ok(request) => self.spawn_start(StartCommand::Install(request), cx),
            Err(error) => {
                self.state.command_error = Some(error);
                cx.notify();
            }
        }
    }

    fn start_launch(&mut self, cx: &mut Context<Self>) {
        if !self.state.can_start() {
            return;
        }
        let request = self.install_request(cx).and_then(|install| {
            let username = self.username.read(cx).value().to_string();
            let account = OfflineAccount::new(username).map_err(|error| error.to_string())?;
            Ok(GameLaunchRequest {
                install,
                account,
                additional_jvm_arguments: Vec::new(),
                additional_game_arguments: Vec::new(),
                resolution: None,
                demo: false,
            })
        });
        match request {
            Ok(request) => self.spawn_start(StartCommand::Launch(request), cx),
            Err(error) => {
                self.state.command_error = Some(error);
                cx.notify();
            }
        }
    }

    fn spawn_start(&mut self, command: StartCommand, cx: &mut Context<Self>) {
        self.state.begin_command();
        cx.notify();
        let services = self.services.clone();
        self._command_task = Some(cx.spawn(async move |this, cx| {
            let manager = services.tasks.clone();
            let command_manager = manager.clone();
            let result = services.runtime.spawn(async move {
                match command {
                    StartCommand::Install(request) => command_manager.start_install(request).await,
                    StartCommand::Launch(request) => command_manager.start_launch(request).await,
                }
            });
            let result = result.await;
            match result {
                Ok(Ok(id)) => {
                    let snapshot = manager.snapshot(id).await;
                    let logs = manager.logs(id).await;
                    let _ = this.update(cx, |view, cx| {
                        view.state.complete_start(id, snapshot, logs);
                        cx.notify();
                    });
                }
                Ok(Err(error)) => {
                    let _ = this.update(cx, |view, cx| {
                        view.state.fail_command(error.to_string());
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = this.update(cx, |view, cx| {
                        view.state.fail_command(format!("后台命令失败: {error}"));
                        cx.notify();
                    });
                }
            }
        }));
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.state.active_id() else {
            return;
        };
        if !self.state.can_cancel() {
            return;
        }
        self.state.begin_command();
        cx.notify();
        let services = self.services.clone();
        self._command_task = Some(cx.spawn(async move |this, cx| {
            let manager = services.tasks.clone();
            let cancel_manager = manager.clone();
            let result = services
                .runtime
                .spawn(async move { cancel_manager.cancel(id).await })
                .await;
            let snapshot = manager.snapshot(id).await;
            let _ = this.update(cx, |view, cx| {
                view.state.finish_command();
                if let Some(snapshot) = snapshot {
                    view.state.apply_snapshot(snapshot);
                }
                if let Err(error) = result {
                    view.state.command_error = Some(format!("取消命令失败: {error}"));
                }
                cx.notify();
            });
        }));
    }
}

impl Render for LauncherView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.state.snapshot.clone();
        let (progress, indeterminate) = task_progress(snapshot.as_ref());
        let current = current_item(snapshot.as_ref());
        let error = self.state.command_error.clone().or_else(|| {
            snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.error.clone())
        });
        let can_start = self.state.can_start();
        let can_cancel = self.state.can_cancel();

        div()
            .flex()
            .flex_col()
            .size_full()
            .p_4()
            .gap_4()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_lg().child("RMCL · Vanilla"))
                    .child(status_label(snapshot.as_ref())),
            )
            .child(
                Form::new()
                    .child(
                        Field::new()
                            .label("Minecraft 目录")
                            .child(Input::new(&self.minecraft_root)),
                    )
                    .child(
                        Field::new()
                            .label("版本 ID")
                            .child(Input::new(&self.version_id)),
                    )
                    .child(
                        Field::new()
                            .label("离线玩家名")
                            .child(Input::new(&self.username)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("install")
                            .label("安装")
                            .disabled(!can_start)
                            .on_click(cx.listener(|this, _, _, cx| this.start_install(cx))),
                    )
                    .child(
                        Button::new("launch")
                            .primary()
                            .label("安装并启动")
                            .disabled(!can_start)
                            .on_click(cx.listener(|this, _, _, cx| this.start_launch(cx))),
                    )
                    .child(
                        Button::new("cancel")
                            .label("取消 / 停止")
                            .disabled(!can_cancel)
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        Progress::new("task-progress")
                            .value(progress)
                            .loading(indeterminate),
                    )
                    .child(phase_label(snapshot.as_ref()))
                    .child(current)
                    .when_some(error, |this, error| {
                        this.child(div().text_color(cx.theme().danger).child(error))
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child("游戏日志")
                    .child(log_summary(snapshot.as_ref())),
            )
            .child(
                div()
                    .id("game-logs")
                    .flex()
                    .flex_col()
                    .flex_grow(1.0)
                    .gap_1()
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().border)
                    .p_2()
                    .text_xs()
                    .track_scroll(&self.log_scroll)
                    .children(self.state.logs.iter().map(|log| {
                        let stream = match log.stream {
                            LogStream::Stdout => "out",
                            LogStream::Stderr => "err",
                        };
                        div().child(format!("[{stream}] {}", log.text))
                    })),
            )
    }
}

fn status_label(snapshot: Option<&TaskSnapshot>) -> String {
    match snapshot.map(|snapshot| snapshot.status) {
        None => "就绪".to_owned(),
        Some(TaskStatus::Running) => "运行中".to_owned(),
        Some(TaskStatus::Cancelling) => "正在取消".to_owned(),
        Some(TaskStatus::Completed) => "安装完成".to_owned(),
        Some(TaskStatus::Cancelled) => "已取消".to_owned(),
        Some(TaskStatus::Failed) => "任务失败".to_owned(),
        Some(TaskStatus::Exited) => "游戏已退出".to_owned(),
    }
}

fn phase_label(snapshot: Option<&TaskSnapshot>) -> String {
    let Some(phase) = snapshot.and_then(|snapshot| snapshot.phase) else {
        return "选择版本后可安装或启动".to_owned();
    };
    match phase {
        TaskPhase::Install(InstallPhase::ResolvingVersion) => "解析版本 manifest",
        TaskPhase::Install(InstallPhase::ReadingVersion) => "读取版本 JSON",
        TaskPhase::Install(InstallPhase::PersistingVersion) => "保存版本 JSON",
        TaskPhase::Install(InstallPhase::PlanningBase) => "规划游戏文件",
        TaskPhase::Install(InstallPhase::DownloadingBase) => "下载游戏文件",
        TaskPhase::Install(InstallPhase::ReadingAssetIndex) => "读取资源索引",
        TaskPhase::Install(InstallPhase::PlanningAssets) => "规划资源对象",
        TaskPhase::Install(InstallPhase::DownloadingAssets) => "下载资源对象",
        TaskPhase::Install(InstallPhase::MergingNatives) => "合并 natives",
        TaskPhase::Install(InstallPhase::Completed) => "安装已完成",
        TaskPhase::CheckingJava => "检查 Java",
        TaskPhase::InstallingJava => "安装 Java",
        TaskPhase::PreparingLaunch => "准备启动参数",
        TaskPhase::StartingGame => "创建游戏进程",
        TaskPhase::RunningGame => "游戏运行中",
    }
    .to_owned()
}

fn current_item(snapshot: Option<&TaskSnapshot>) -> String {
    let Some(progress) = snapshot.and_then(|snapshot| snapshot.progress.as_ref()) else {
        return String::new();
    };
    match progress {
        TaskProgress::Download(progress) => format!(
            "{} / {} 个文件 · {}",
            progress.completed_files,
            progress.total_files,
            progress.current.display()
        ),
        TaskProgress::Natives {
            current,
            completed_archives,
            total_archives,
        } => format!(
            "{completed_archives} / {total_archives} 个 native JAR · {}",
            current.display()
        ),
    }
}

fn task_progress(snapshot: Option<&TaskSnapshot>) -> (f32, bool) {
    let Some(snapshot) = snapshot else {
        return (0.0, false);
    };
    let value = match snapshot.progress.as_ref() {
        Some(TaskProgress::Download(progress)) if progress.total_files > 0 => {
            progress.completed_files as f32 / progress.total_files as f32 * 100.0
        }
        Some(TaskProgress::Natives {
            completed_archives,
            total_archives,
            ..
        }) if *total_archives > 0 => *completed_archives as f32 / *total_archives as f32 * 100.0,
        _ if snapshot.status == TaskStatus::Completed => 100.0,
        _ => 0.0,
    };
    let indeterminate = matches!(
        snapshot.status,
        TaskStatus::Running | TaskStatus::Cancelling
    ) && value == 0.0;
    (value, indeterminate)
}

fn log_summary(snapshot: Option<&TaskSnapshot>) -> String {
    snapshot.map_or_else(String::new, |snapshot| format!("{} 行", snapshot.log_lines))
}
