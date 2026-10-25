use std::collections::VecDeque;

use rmcl_app::{TaskId, TaskSnapshot, TaskStatus};
use rmcl_launch::GameLog;

const UI_LOG_CAPACITY: usize = 256;

pub struct LauncherState {
    selected_id: Option<TaskId>,
    pub snapshot: Option<TaskSnapshot>,
    pub logs: VecDeque<GameLog>,
    last_log_line: u64,
    pub command_pending: bool,
    pub command_error: Option<String>,
    pub recovered_after_lag: bool,
    terminal_notified: Option<TaskId>,
}

impl LauncherState {
    pub fn new() -> Self {
        Self {
            selected_id: None,
            snapshot: None,
            logs: VecDeque::new(),
            last_log_line: 0,
            command_pending: false,
            command_error: None,
            recovered_after_lag: false,
            terminal_notified: None,
        }
    }

    pub fn active_id(&self) -> Option<TaskId> {
        self.selected_id
    }

    pub fn can_start(&self) -> bool {
        !self.command_pending
            && self
                .snapshot
                .as_ref()
                .is_none_or(|snapshot| snapshot.status.is_terminal())
    }

    pub fn can_cancel(&self) -> bool {
        !self.command_pending
            && self
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.status == TaskStatus::Running)
    }

    pub fn begin_command(&mut self) {
        self.command_pending = true;
        self.command_error = None;
        self.recovered_after_lag = false;
    }

    pub fn complete_start(
        &mut self,
        id: TaskId,
        snapshot: Option<TaskSnapshot>,
        logs: Option<Vec<GameLog>>,
    ) {
        self.last_log_line = snapshot.as_ref().map_or(0, |snapshot| snapshot.log_lines);
        if let Some(logs) = &logs {
            self.last_log_line = self.last_log_line.max(logs.len() as u64);
        }
        self.selected_id = Some(id);
        self.snapshot = snapshot;
        self.logs = logs.unwrap_or_default().into();
        self.command_pending = false;
        self.command_error = None;
        self.recovered_after_lag = false;
    }

    pub fn fail_command(&mut self, error: String) {
        self.command_pending = false;
        self.command_error = Some(error);
    }

    pub fn finish_command(&mut self) {
        self.command_pending = false;
    }

    pub fn apply_snapshot(&mut self, snapshot: TaskSnapshot) -> bool {
        let accepted = if self.selected_id == Some(snapshot.id) {
            true
        } else {
            self.snapshot
                .as_ref()
                .is_none_or(|current| current.status.is_terminal())
        };
        if !accepted {
            return false;
        }
        if let Some(current) = &self.snapshot
            && current.id == snapshot.id
        {
            if current.status.is_terminal() && current.status != snapshot.status {
                return false;
            }
            if current.status == TaskStatus::Cancelling && snapshot.status == TaskStatus::Running {
                return false;
            }
        }
        if self.selected_id != Some(snapshot.id) {
            self.logs.clear();
            self.last_log_line = 0;
        }
        self.selected_id = Some(snapshot.id);
        self.snapshot = Some(snapshot);
        self.recovered_after_lag = false;
        true
    }

    pub fn apply_log(&mut self, task_id: TaskId, line: u64, log: GameLog) -> bool {
        if self.selected_id != Some(task_id) || line <= self.last_log_line {
            return false;
        }
        self.last_log_line = line;
        if self.logs.len() == UI_LOG_CAPACITY {
            self.logs.pop_front();
        }
        self.logs.push_back(log);
        true
    }

    pub fn recover(&mut self, snapshot: TaskSnapshot, logs: Vec<GameLog>) {
        self.last_log_line = snapshot.log_lines.max(logs.len() as u64);
        self.selected_id = Some(snapshot.id);
        self.snapshot = Some(snapshot);
        self.logs = logs.into();
        self.recovered_after_lag = true;
    }

    /// Returns true only for the first terminal observation of a task.
    pub fn mark_terminal_notified(&mut self, id: TaskId) -> bool {
        if self.terminal_notified == Some(id) {
            false
        } else {
            self.terminal_notified = Some(id);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcl_app::{InstallPhase, TaskKind, TaskPhase};
    use rmcl_launch::LogStream;

    fn snapshot(id: u64, status: TaskStatus) -> TaskSnapshot {
        TaskSnapshot {
            id: TaskId::from_raw(id),
            kind: TaskKind::GameLaunch,
            status,
            phase: Some(TaskPhase::Install(InstallPhase::DownloadingBase)),
            progress: None,
            error: None,
            exit: None,
            stop_requested: false,
            log_lines: 0,
        }
    }

    fn log(text: &str) -> GameLog {
        GameLog {
            stream: LogStream::Stdout,
            text: text.to_owned(),
        }
    }

    #[test]
    fn terminal_snapshot_allows_the_next_task_without_mixing_logs() {
        let mut state = LauncherState::new();
        state.complete_start(
            TaskId::from_raw(1),
            Some(snapshot(1, TaskStatus::Exited)),
            Some(vec![log("old")]),
        );
        assert!(state.can_start());

        assert!(state.apply_snapshot(snapshot(2, TaskStatus::Running)));

        assert_eq!(state.active_id(), Some(TaskId::from_raw(2)));
        assert!(state.logs.is_empty());
        assert!(!state.apply_log(TaskId::from_raw(1), 1, log("stale")));
    }

    #[test]
    fn running_task_does_not_accept_other_ids() {
        let mut state = LauncherState::new();
        state.complete_start(
            TaskId::from_raw(1),
            Some(snapshot(1, TaskStatus::Running)),
            None,
        );

        assert!(!state.apply_snapshot(snapshot(2, TaskStatus::Completed)));
        assert!(!state.can_start());
        assert!(state.can_cancel());
    }

    #[test]
    fn log_buffer_is_bounded() {
        let mut state = LauncherState::new();
        state.complete_start(
            TaskId::from_raw(1),
            Some(snapshot(1, TaskStatus::Running)),
            None,
        );
        for index in 0..(UI_LOG_CAPACITY + 20) {
            assert!(state.apply_log(
                TaskId::from_raw(1),
                index as u64 + 1,
                log(&format!("line-{index}"))
            ));
        }

        assert_eq!(state.logs.len(), UI_LOG_CAPACITY);
        assert_eq!(state.logs.front().unwrap().text, "line-20");
    }

    #[test]
    fn terminal_notification_is_emitted_once_per_task() {
        let mut state = LauncherState::new();
        let id = TaskId::from_raw(7);

        assert!(state.mark_terminal_notified(id));
        assert!(!state.mark_terminal_notified(id));
        assert!(state.mark_terminal_notified(TaskId::from_raw(8)));
    }
}
