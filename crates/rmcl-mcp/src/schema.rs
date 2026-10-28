use rmcl_app::{InstalledVersion, JavaRuntime, LocalContent, TaskSnapshot, TaskStatus};
use rmcl_launch::{GameLog, LogStream};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::error::invalid_input;

pub fn instance_summary(version: &InstalledVersion) -> Result<Value, rmcp::ErrorData> {
    Ok(json!({
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
        "revision": instance_revision(version)?,
    }))
}

pub fn instance_revision(version: &InstalledVersion) -> Result<String, rmcp::ErrorData> {
    let json_path = version
        .version_dir
        .join(format!("{}.json", version.id.as_str()));
    let manifest_path = version.version_dir.join("rmcl.instance.toml");
    let mut hasher = Sha256::new();
    hasher.update(std::fs::read(json_path).map_err(crate::error::internal_error)?);
    match std::fs::read(manifest_path) {
        Ok(manifest) => {
            hasher.update([0]);
            hasher.update(manifest);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(crate::error::internal_error(error)),
    }
    Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
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
    fn instance_revision_covers_version_json_and_manifest() {
        let temporary = tempfile::TempDir::new().unwrap();
        let root = temporary.path().join("minecraft");
        let version_dir = root.join("versions/example");
        std::fs::create_dir_all(&version_dir).unwrap();
        let json_path = version_dir.join("example.json");
        let manifest_path = version_dir.join("rmcl.instance.toml");
        std::fs::write(&json_path, br#"{"id":"example","type":"release"}"#).unwrap();
        let instance = rmcl_app::discover_installed_versions(&root)
            .unwrap()
            .remove(0);

        let without_manifest = instance_revision(&instance).unwrap();
        std::fs::write(&manifest_path, b"name = \"Example\"\n").unwrap();
        let with_manifest = instance_revision(&instance).unwrap();
        assert_ne!(without_manifest, with_manifest);

        std::fs::write(&manifest_path, b"name = \"Renamed\"\n").unwrap();
        let renamed = instance_revision(&instance).unwrap();
        assert_ne!(with_manifest, renamed);

        std::fs::write(
            &json_path,
            br#"{"id":"example","type":"release","releaseTime":"2026-01-01T00:00:00Z"}"#,
        )
        .unwrap();
        assert_ne!(renamed, instance_revision(&instance).unwrap());
    }

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
