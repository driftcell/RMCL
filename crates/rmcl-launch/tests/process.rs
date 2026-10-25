use std::path::PathBuf;
#[cfg(unix)]
use std::time::Duration;

use rmcl_domain::{Architecture, OperatingSystem, Platform};
use rmcl_instance::MinecraftLayout;
use rmcl_java::{JavaRuntime, JavaRuntimeSource};
#[cfg(unix)]
use rmcl_launch::{GameEvent, LogStream};
use rmcl_launch::{GameProcess, LaunchAccount, LaunchOptions, LaunchPlan};
use rmcl_minecraft::GameVersion;
#[cfg(unix)]
use tempfile::TempDir;
use uuid::Uuid;

#[cfg(unix)]
fn write_script(content: &str) -> (TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let directory = TempDir::new().unwrap();
    let path = directory.path().join("java");
    std::fs::write(&path, content).unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).unwrap();
    (directory, path)
}

fn account() -> LaunchAccount {
    LaunchAccount {
        username: "Steve".to_owned(),
        uuid: Uuid::parse_str("5627dd98-e6be-3c21-b8a8-e92344183641").unwrap(),
        access_token: "secret-token".to_owned(),
        user_type: "msa".to_owned(),
        user_properties: "{}".to_owned(),
    }
}

fn platform() -> Platform {
    Platform::new(OperatingSystem::Linux, "6.1", "x86_64")
}

fn runtime(executable: PathBuf) -> JavaRuntime {
    JavaRuntime {
        executable,
        version: "21.0.7".to_owned(),
        major_version: 21,
        operating_system: OperatingSystem::Linux,
        architecture: Architecture::X86_64,
        vendor: None,
        source: JavaRuntimeSource::Managed,
    }
}

fn version() -> GameVersion {
    GameVersion::from_json(
        r#"{"id":"test","mainClass":"example.Main","javaVersion":{"component":"java-runtime-delta","majorVersion":21}}"#,
    )
    .unwrap()
}

fn process_plan(executable: PathBuf, working_directory: PathBuf) -> LaunchPlan {
    let version = version();
    let options = LaunchOptions::from_layout(
        &MinecraftLayout::new(working_directory),
        runtime(executable),
        &version,
        account(),
        &platform(),
    )
    .unwrap();
    LaunchPlan::from_options(&version, options).unwrap()
}

#[cfg(unix)]
#[tokio::test]
async fn collects_logs_and_redacts_tokens_before_exit() {
    let (directory, executable) =
        write_script("#!/bin/sh\necho \"out: $*\"\necho \"err: secret-token\" >&2\nexit 0\n");
    let plan = process_plan(executable, directory.path().to_path_buf());
    let mut process = GameProcess::spawn(&plan).unwrap();
    let mut logs = Vec::new();

    let exit = loop {
        match process.next_event().await.unwrap() {
            Some(GameEvent::Output(log)) => logs.push(log),
            Some(GameEvent::Exit(exit)) => break exit,
            None => panic!("event stream ended before an exit event"),
        }
    };

    assert!(exit.success);
    assert_eq!(exit.code, Some(0));
    assert!(logs.iter().any(|log| log.stream == LogStream::Stdout));
    assert!(logs.iter().any(|log| log.stream == LogStream::Stderr));
    let output = logs
        .iter()
        .map(|log| log.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(output.contains("example.Main"));
    assert!(!output.contains("secret-token"));
    assert!(output.contains("<redacted>"));
}

#[cfg(unix)]
#[tokio::test]
async fn kill_returns_an_unsuccessful_exit() {
    let (directory, executable) = write_script("#!/bin/sh\nsleep 60\n");
    let plan = process_plan(executable, directory.path().to_path_buf());
    let mut process = GameProcess::spawn(&plan).unwrap();

    process.kill().await.unwrap();
    let exit = tokio::time::timeout(Duration::from_secs(3), process.wait())
        .await
        .unwrap()
        .unwrap();

    assert!(!exit.success);
}

#[tokio::test]
async fn reports_spawn_errors() {
    let plan = process_plan(PathBuf::from("missing-java-executable"), PathBuf::from("."));
    let result = GameProcess::spawn(&plan);

    assert!(result.is_err());
}
