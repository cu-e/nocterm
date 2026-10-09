//! The terminal tools server an agent starts is the real binary's relay.
#![cfg(unix)]

use std::{
    io::{BufRead as _, BufReader, Write as _},
    path::PathBuf,
    process::{Command, Stdio},
};

use nocterm_acp::{BridgeServer, RelayCommand};
use nocterm_ai::{ToolBridge as _, acp};
use nocterm_core::Paths;

#[test]
fn the_launched_server_answers_through_the_chat_bridge() {
    let dir = tempfile::tempdir().unwrap();
    let bridge = BridgeServer::new(
        Paths::rooted_at(dir.path()),
        RelayCommand {
            program: PathBuf::from(env!("CARGO_BIN_EXE_nocterm")),
            args: vec!["agent-bridge".into()],
        },
    );
    let registration = bridge.register().unwrap();
    let acp::McpServer::Stdio(server) = registration.mcp_server() else {
        panic!("the relay is a stdio server");
    };
    let mut relay = Command::new(&server.command)
        .args(&server.args)
        .env_clear()
        .envs(server.env.iter().map(|v| (&v.name, &v.value)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = relay.stdin.take().unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize"}}"#).unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list"}}"#).unwrap();
    drop(stdin);
    let lines: Vec<serde_json::Value> = BufReader::new(relay.stdout.take().unwrap())
        .lines()
        .map(|line| serde_json::from_str(&line.unwrap()).unwrap())
        .collect();
    assert!(relay.wait().unwrap().success());
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["result"]["serverInfo"]["name"], "nocterm");
    assert!(
        lines[1]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "exec_command")
    );
}
