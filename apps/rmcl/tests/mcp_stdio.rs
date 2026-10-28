use std::{
    collections::BTreeSet,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tempfile::TempDir;

const IO_TIMEOUT: Duration = Duration::from_secs(30);

struct McpProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    messages: Receiver<Result<Value, String>>,
    stdout: Option<thread::JoinHandle<()>>,
    stderr: Option<thread::JoinHandle<String>>,
}

impl McpProcess {
    fn spawn(temporary: &TempDir) -> Self {
        let config = temporary.path().join("config.toml");
        let mut child = Command::new(env!("CARGO_BIN_EXE_rmcl"))
            .args(["mcp", "--stdio", "--config"])
            .arg(&config)
            .current_dir(temporary.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (sender, messages) = std::sync::mpsc::channel();
        let stdout = thread::spawn(move || read_stdout(stdout, sender));
        let stderr = thread::spawn(move || {
            let mut output = String::new();
            BufReader::new(stderr).read_to_string(&mut output).unwrap();
            output
        });
        Self {
            child,
            stdin: Some(stdin),
            messages,
            stdout: Some(stdout),
            stderr: Some(stderr),
        }
    }

    fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().unwrap();
        serde_json::to_writer(&mut *stdin, &message).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }

    fn call(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let message = self
                .messages
                .recv_timeout(IO_TIMEOUT)
                .expect("timed out waiting for the RMCL MCP response")
                .expect("RMCL wrote a non-JSON line to MCP stdout");
            if message["id"] == id {
                return message;
            }
        }
    }

    fn shutdown(&mut self) {
        drop(self.stdin.take());
        let deadline = Instant::now() + IO_TIMEOUT;
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "RMCL did not exit after stdio EOF"
            );
            thread::sleep(Duration::from_millis(20));
        };
        assert!(status.success(), "RMCL MCP exited with {status}");
        self.stdout.take().unwrap().join().unwrap();
        let stderr = self.stderr.take().unwrap().join().unwrap();
        assert!(
            stderr.is_empty(),
            "RMCL MCP wrote unexpected stderr: {stderr}"
        );
    }
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_stdout(stdout: impl Read, sender: Sender<Result<Value, String>>) {
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        let bytes = reader.read_line(&mut line).unwrap();
        if bytes == 0 {
            break;
        }
        let message = serde_json::from_str(&line)
            .map_err(|error| format!("invalid stdout frame {line:?}: {error}"));
        let failed = message.is_err();
        if sender.send(message).is_err() || failed {
            break;
        }
    }
}

#[test]
fn rmcl_binary_serves_mcp_over_stdio_without_a_display() {
    let temporary = TempDir::new().unwrap();
    let mut process = McpProcess::spawn(&temporary);
    let initialized = process.call(
        1,
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "rmcl-stdio-smoke", "version": "1.0"},
        }),
    );
    assert_eq!(initialized["result"]["serverInfo"]["name"], "rmcl");
    process.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));

    let tools = process.call(2, "tools/list", json!({}));
    let tools = tools["result"]["tools"].as_array().unwrap();
    let names = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    for required in [
        "capabilities",
        "instances_list",
        "accounts_list",
        "java_list",
        "tasks_subscribe",
        "tasks_unsubscribe",
    ] {
        assert!(names.contains(required), "missing MCP tool {required}");
    }
    for tool in tools {
        assert!(
            tool["outputSchema"].is_object(),
            "missing outputSchema: {tool}"
        );
        assert!(
            tool["annotations"].is_object(),
            "missing annotations: {tool}"
        );
    }

    let capabilities = process.call(
        3,
        "tools/call",
        json!({"name": "capabilities", "arguments": {}}),
    );
    let capabilities = &capabilities["result"]["structuredContent"];
    assert_eq!(capabilities["access"], "read_write");
    assert!(
        capabilities["transports"]
            .as_array()
            .unwrap()
            .contains(&json!("stdio"))
    );

    let subscribed = process.call(
        4,
        "tools/call",
        json!({"name": "tasks_subscribe", "arguments": {}}),
    );
    let subscription_id = subscribed["result"]["structuredContent"]["subscription_id"]
        .as_u64()
        .unwrap();
    let unsubscribed = process.call(
        5,
        "tools/call",
        json!({
            "name": "tasks_unsubscribe",
            "arguments": {"subscription_id": subscription_id},
        }),
    );
    assert_eq!(
        unsubscribed["result"]["structuredContent"]["unsubscribed"],
        true
    );
    process.shutdown();
}
