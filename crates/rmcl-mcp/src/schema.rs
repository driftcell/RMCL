use rmcl_app::{InstalledVersion, JavaRuntime, LocalContent, TaskSnapshot, TaskStatus};
use rmcl_launch::{GameLog, LogStream};
use serde_json::{Value, json};

use crate::error::invalid_input;

pub fn instance_summary(version: &InstalledVersion) -> Value {
    json!({
        "id": version.id.as_str(),
        "name": version.display_name(),
        "minecraft_version": version.minecraft_version.clone(),
        "version_type": version.version_type.clone(),
        "release_time": version.release_time.clone(),
        "game_directory": version.game_dir.display().to_string(),
        "isolated": version.is_isolated(),
        "legacy": version.is_legacy(),
        "loader": version.loader.as_ref().map(|loader| json!({
            "kind": loader.kind.as_str(),
            "version": loader.version.clone(),
        })),
        "java": version.java.as_ref().map(|path| path.display().to_string()),
        "max_memory_mb": version.max_memory_mb,
        "window": match (version.window_width, version.window_height) {
            (Some(width), Some(height)) => Some(json!({ "width": width, "height": height })),
            _ => None,
        },
        "manifest_present": version.manifest_present,
        "manifest_error": version.manifest_error.clone(),
        "client_jar_present": version.client_jar_present,
    })
}

pub fn content_entry(item: &LocalContent) -> Value {
    json!({
        "kind": item.kind.modrinth_project_type(),
        "name": item.display_name(),
        "relative_path": item.relative_path.as_str(),
        "size": item.size,
        "enabled": item.enabled,
        "is_directory": item.is_directory,
        "can_toggle": item.can_toggle(),
    })
}

pub fn java_runtime(runtime: &JavaRuntime) -> Value {
    json!({
        "executable": runtime.executable.display().to_string(),
        "version": runtime.version.clone(),
        "major_version": runtime.major_version,
        "operating_system": runtime.operating_system.as_str(),
        "architecture": runtime.architecture.as_str(),
        "vendor": runtime.vendor.clone(),
        "source": format!("{:?}", runtime.source),
    })
}

pub fn task_snapshot(snapshot: &TaskSnapshot) -> Value {
    json!({
        "id": snapshot.id.get(),
        "kind": format!("{:?}", snapshot.kind),
        "subject": snapshot.subject.as_ref().map(|subject| format!("{subject:?}")),
        "status": task_status(snapshot.status),
        "phase": snapshot.phase.map(|phase| format!("{phase:?}")),
        "progress": snapshot.progress.as_ref().map(|progress| format!("{progress:?}")),
        "error": snapshot.error.clone(),
        "exit": snapshot.exit.as_ref().map(|exit| format!("{exit:?}")),
        "stop_requested": snapshot.stop_requested,
        "log_lines": snapshot.log_lines,
    })
}

fn task_status(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Running => "running",
        TaskStatus::Cancelling => "cancelling",
        TaskStatus::Completed => "completed",
        TaskStatus::Cancelled => "cancelled",
        TaskStatus::Failed => "failed",
        TaskStatus::Exited => "exited",
    }
}

pub fn game_log(log: &GameLog) -> Value {
    json!({
        "stream": match log.stream {
            LogStream::Stdout => "stdout",
            LogStream::Stderr => "stderr",
        },
        "text": log.text.clone(),
    })
}

pub fn log_page(
    logs: &[GameLog],
    cursor: Option<usize>,
    limit: Option<usize>,
) -> Result<Value, rmcp::ErrorData> {
    let limit = limit.unwrap_or(200).clamp(1, 1000);
    let start = cursor.unwrap_or(0);
    if start > logs.len() {
        return Err(invalid_input("log cursor is past the end of the log"));
    }
    let end = start.saturating_add(limit).min(logs.len());
    let next_cursor = (end < logs.len()).then_some(end);
    Ok(json!({
        "items": logs[start..end].iter().map(game_log).collect::<Vec<_>>(),
        "next_cursor": next_cursor,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_pages_are_bounded_and_report_the_next_cursor() {
        let logs = (0..3)
            .map(|index| GameLog {
                stream: LogStream::Stdout,
                text: format!("line {index}"),
            })
            .collect::<Vec<_>>();

        let first = log_page(&logs, None, Some(2)).unwrap();
        assert_eq!(first["items"].as_array().unwrap().len(), 2);
        assert_eq!(first["next_cursor"], 2);
        let second = log_page(&logs, Some(2), Some(2)).unwrap();
        assert_eq!(second["items"].as_array().unwrap().len(), 1);
        assert_eq!(second["next_cursor"], Value::Null);
        assert!(log_page(&logs, Some(4), None).is_err());
    }
}
