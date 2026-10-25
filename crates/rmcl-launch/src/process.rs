use std::{fmt, io, process::ExitStatus, sync::Arc};

use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::{Child, Command},
    sync::mpsc,
    task::JoinHandle,
};

use crate::{LaunchError, LaunchPlan};

#[derive(Clone)]
pub struct LogRedactor {
    secret: Option<Arc<str>>,
}

impl fmt::Debug for LogRedactor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LogRedactor")
            .field("has_secret", &self.secret.is_some())
            .finish()
    }
}

impl LogRedactor {
    pub fn new(secret: Option<String>) -> Self {
        let secret = secret.filter(|secret| !secret.is_empty()).map(Arc::from);
        Self { secret }
    }

    pub fn redact(&self, text: &str) -> String {
        match &self.secret {
            Some(secret) => text.replace(secret.as_ref(), "<redacted>"),
            None => text.to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameLog {
    pub stream: LogStream,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameEvent {
    Output(GameLog),
    Exit(GameExit),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameExit {
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub success: bool,
}

pub struct GameProcess {
    child: Child,
    receiver: mpsc::Receiver<GameLog>,
    tasks: Vec<JoinHandle<io::Result<()>>>,
    exit_status: Option<ExitStatus>,
    exit_reported: bool,
}

impl GameProcess {
    /// Spawns the game without a shell and begins pumping both output streams.
    pub fn spawn(plan: &LaunchPlan) -> Result<Self, LaunchError> {
        let mut command = Command::new(&plan.java);
        command
            .args(&plan.jvm_arguments)
            .arg(&plan.main_class)
            .args(&plan.game_arguments)
            .current_dir(&plan.working_directory)
            .envs(&plan.environment)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|source| LaunchError::Spawn {
            path: plan.java.display().to_string(),
            source,
        })?;
        let stdout = child.stdout.take().ok_or_else(|| LaunchError::Io {
            path: plan.java.display().to_string(),
            source: io::Error::new(io::ErrorKind::BrokenPipe, "game stdout is unavailable"),
        })?;
        let stderr = child.stderr.take().ok_or_else(|| LaunchError::Io {
            path: plan.java.display().to_string(),
            source: io::Error::new(io::ErrorKind::BrokenPipe, "game stderr is unavailable"),
        })?;
        let (sender, receiver) = mpsc::channel(64);
        let redactor = plan.redactor();
        let tasks = vec![
            spawn_log_pump(stdout, LogStream::Stdout, sender.clone(), redactor.clone()),
            spawn_log_pump(stderr, LogStream::Stderr, sender.clone(), redactor),
        ];
        drop(sender);
        Ok(Self {
            child,
            receiver,
            tasks,
            exit_status: None,
            exit_reported: false,
        })
    }

    /// Waits for the next log or exit event, draining logs before reporting exit.
    ///
    /// Returns `None` only after an exit event has already been returned.
    pub async fn next_event(&mut self) -> Result<Option<GameEvent>, LaunchError> {
        loop {
            if self.exit_reported {
                return Ok(None);
            }
            if self.exit_status.is_some() {
                match self.receiver.recv().await {
                    Some(log) => return Ok(Some(GameEvent::Output(log))),
                    None => {
                        self.finish_tasks().await?;
                        self.exit_reported = true;
                        let status = self.exit_status.ok_or_else(|| {
                            LaunchError::ProcessTask("game exited without a status".to_owned())
                        })?;
                        return Ok(Some(GameEvent::Exit(exit_event(status))));
                    }
                }
            }

            tokio::select! {
                log = self.receiver.recv() => {
                    match log {
                        Some(log) => return Ok(Some(GameEvent::Output(log))),
                        None => {
                            self.exit_status = Some(self.child.wait().await.map_err(|source| io_error(&self.child, source))?);
                            self.finish_tasks().await?;
                            self.exit_reported = true;
                            let status = self.exit_status.ok_or_else(|| {
                                LaunchError::ProcessTask("game exited without a status".to_owned())
                            })?;
                            return Ok(Some(GameEvent::Exit(exit_event(status))));
                        }
                    }
                }
                status = self.child.wait() => {
                    self.exit_status = Some(status.map_err(|source| io_error(&self.child, source))?);
                }
            }
        }
    }

    /// Drains and discards remaining log events until the game exits.
    pub async fn wait(&mut self) -> Result<GameExit, LaunchError> {
        loop {
            match self.next_event().await? {
                Some(GameEvent::Exit(exit)) => return Ok(exit),
                Some(GameEvent::Output(_)) => {}
                None => {
                    return Err(LaunchError::ProcessTask(
                        "game event stream closed before exit".to_owned(),
                    ));
                }
            }
        }
    }

    /// Kills the child; use `wait` or `next_event` to observe the resulting exit.
    pub async fn kill(&mut self) -> Result<(), LaunchError> {
        self.child
            .kill()
            .await
            .map_err(|source| io_error(&self.child, source))
    }

    async fn finish_tasks(&mut self) -> Result<(), LaunchError> {
        for task in self.tasks.drain(..) {
            match task.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    return Err(LaunchError::Io {
                        path: "game log stream".to_owned(),
                        source: error,
                    });
                }
                Err(error) => return Err(LaunchError::ProcessTask(error.to_string())),
            }
        }
        Ok(())
    }
}

fn spawn_log_pump<R>(
    reader: R,
    stream: LogStream,
    sender: mpsc::Sender<GameLog>,
    redactor: LogRedactor,
) -> JoinHandle<io::Result<()>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut reader = BufReader::new(reader);
        loop {
            let mut bytes = Vec::new();
            let read = reader.read_until(b'\n', &mut bytes).await?;
            if read == 0 {
                break;
            }
            if bytes.last() == Some(&b'\n') {
                bytes.pop();
            }
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
            let text = redactor.redact(&String::from_utf8_lossy(&bytes));
            if sender.send(GameLog { stream, text }).await.is_err() {
                break;
            }
        }
        Ok(())
    })
}

fn exit_event(status: ExitStatus) -> GameExit {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        GameExit {
            code: status.code(),
            signal: status.signal(),
            success: status.success(),
        }
    }
    #[cfg(not(unix))]
    {
        GameExit {
            code: status.code(),
            signal: None,
            success: status.success(),
        }
    }
}

fn io_error(child: &Child, source: io::Error) -> LaunchError {
    LaunchError::Io {
        path: child
            .id()
            .map_or_else(|| "game process".to_owned(), |id| id.to_string()),
        source,
    }
}
