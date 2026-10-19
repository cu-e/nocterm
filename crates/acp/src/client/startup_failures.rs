//! Agent startup reports exact failures and never leaves a process behind.
use super::*;
use nocterm_ai::{AgentLaunch, ConnectionCancellation};

const ECHO_AGENT: &str = r#"
import json, sys
version = int(sys.argv[1])
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    result = {'protocolVersion': version, 'agentCapabilities': {}}
    print(json.dumps({'jsonrpc': '2.0', 'id': req['id'], 'result': result}), flush=True)
"#;

fn request(
    command: &str,
    args: Vec<String>,
    directory: &std::path::Path,
    cancellation: ConnectionCancellation,
) -> ConnectRequest {
    ConnectRequest {
        launch: AgentLaunch {
            id: "startup".into(),
            name: "Startup".into(),
            command: command.into(),
            args,
            env: Default::default(),
            inherit_env: Vec::new(),
        },
        working_directory: directory.to_owned(),
        terminal_auth: false,
        sandbox: None,
        resources: Default::default(),
        cancellation,
    }
}

fn scripted(version: u32, directory: &std::path::Path) -> ConnectRequest {
    request(
        "/usr/bin/python3",
        vec!["-c".into(), ECHO_AGENT.into(), version.to_string()],
        directory,
        Default::default(),
    )
}

#[test]
fn a_cancelled_startup_spawns_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let cancellation = ConnectionCancellation::default();
    cancellation.cancel();
    let error = futures::executor::block_on(AcpConnector::unmanaged().connect(request(
        "/nonexistent/agent-that-would-fail-to-spawn",
        Vec::new(),
        directory.path(),
        cancellation,
    )))
    .err()
    .unwrap();
    assert!(matches!(error, AgentError::Io(message) if message == "Agent startup cancelled"));
}

#[test]
fn cancelling_while_the_agent_initializes_stops_it() {
    let directory = tempfile::tempdir().unwrap();
    let cancellation = ConnectionCancellation::default();
    let trigger = cancellation.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        trigger.cancel();
    });
    let error = futures::executor::block_on(AcpConnector::unmanaged().connect(request(
        "/usr/bin/python3",
        vec!["-c".into(), "import time; time.sleep(60)".into()],
        directory.path(),
        cancellation,
    )))
    .err()
    .unwrap();
    assert!(matches!(error, AgentError::Io(message) if message == "Agent startup cancelled"));
}

#[test]
fn an_unsupported_protocol_version_is_refused() {
    let directory = tempfile::tempdir().unwrap();
    let error = futures::executor::block_on(
        AcpConnector::unmanaged().connect(scripted(2, directory.path())),
    )
    .err()
    .unwrap();
    assert!(
        matches!(error, AgentError::Io(message) if message == "Unsupported ACP protocol version")
    );
}

#[test]
fn the_default_connector_initializes_a_scripted_agent() {
    let directory = tempfile::tempdir().unwrap();
    for connector in [AcpConnector::default(), AcpConnector::unmanaged()] {
        let connection =
            futures::executor::block_on(connector.connect(scripted(1, directory.path()))).unwrap();
        assert_eq!(connection.info.name, "Startup");
        futures::executor::block_on(connection.commands.shutdown_gracefully()).unwrap();
    }
}
