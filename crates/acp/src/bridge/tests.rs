use super::*;
use std::io::Read;

/// Reads like `Read::read`, retrying when a signal meant for another
/// test (a reaped child, say) interrupts the timed socket read.
fn read_uninterrupted(socket: &mut Socket, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        match socket.read(buf) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            result => return result,
        }
    }
}
fn bridge(dir: &tempfile::TempDir) -> BridgeServer {
    BridgeServer::new(
        Paths::rooted_at(dir.path()),
        RelayCommand {
            program: std::env::current_exe().unwrap(),
            args: vec!["agent-bridge".into()],
        },
    )
}
fn variable(registration: &BridgeRegistration, name: &str) -> String {
    let (_, value) = registration
        .launch()
        .env
        .iter()
        .find(|(key, _)| key == name)
        .unwrap();
    value.clone()
}
fn endpoint(registration: &BridgeRegistration) -> String {
    variable(registration, ENDPOINT_VARIABLE)
}
fn token(registration: &BridgeRegistration) -> String {
    variable(registration, TOKEN_VARIABLE)
}
#[test]
fn launches_the_relay_under_the_chat_server_name() {
    let dir = tempfile::tempdir().unwrap();
    let registration = bridge(&dir).register().unwrap();
    let nocterm_ai::acp::McpServer::Stdio(server) = registration.mcp_server() else {
        panic!("stdio relay");
    };
    assert_eq!(server.name, format!("nocterm-{}", registration.id));
    assert_eq!(server.command, std::env::current_exe().unwrap());
    assert_eq!(server.args, ["agent-bridge"]);
    let names: Vec<_> = server.env.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, [ENDPOINT_VARIABLE, TOKEN_VARIABLE]);
}
#[test]
fn refuses_a_registration_whose_relay_program_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let server = BridgeServer::new(
        Paths::rooted_at(dir.path()),
        RelayCommand {
            program: dir.path().join("nocterm (deleted)"),
            args: vec!["agent-bridge".into()],
        },
    );
    let error = server.register().err().unwrap();
    assert!(error.contains("nocterm (deleted)"), "{error}");
    assert!(error.contains("Restart nocterm"), "{error}");
}
fn authenticated(registration: &BridgeRegistration) -> (Socket, BufReader<Socket>) {
    let mut socket =
        Socket::connect(endpoint(registration).strip_prefix("unix:").unwrap()).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    writeln!(
        socket,
        "{}",
        serde_json::json!({"token":token(registration)})
    )
    .unwrap();
    let reader = BufReader::new(socket.try_clone().unwrap());
    (socket, reader)
}
fn initialize(socket: &mut Socket, reader: &mut BufReader<Socket>) {
    writeln!(socket,"{}",serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1"}}})).unwrap();
    let response: Value =
        serde_json::from_str(&bounded_line(reader, 1024 * 1024).unwrap().unwrap()).unwrap();
    assert_eq!(response["id"], 1);
    assert_eq!(response["result"]["serverInfo"]["name"], "nocterm");
}
#[test]
fn authenticates_routes_and_revokes_pending_calls() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let server = bridge(&dir);
    let mut registration = server.register().unwrap();
    let endpoint = endpoint(&registration);
    let path = endpoint.strip_prefix("unix:").unwrap();
    assert_eq!(
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let (mut socket, mut reader) = authenticated(&registration);
    initialize(&mut socket, &mut reader);
    writeln!(socket,"{}",serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_terminals","arguments":{}}})).unwrap();
    let call = server.calls().recv_blocking().unwrap();
    assert_eq!(call.registration_id, registration.id);
    assert!(matches!(call.call, nocterm_ai::TerminalCall::ListTerminals));
    call.respond
        .send(Ok(serde_json::json!([{"id":"t1"}])))
        .unwrap();
    let response: Value =
        serde_json::from_str(&bounded_line(&mut reader, 1024 * 1024).unwrap().unwrap()).unwrap();
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("t1")
    );
    writeln!(socket,"{}",serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_terminals"}})).unwrap();
    let pending = server.calls().recv_blocking().unwrap();
    registration.revoke();
    assert!(
        bounded_line(&mut reader, 1024 * 1024)
            .unwrap_or(None)
            .is_none()
    );
    for _ in 0..100 {
        if pending.respond.is_canceled() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(pending.respond.is_canceled());
}
#[test]
fn rejects_bad_tokens_and_limits_thread_connections() {
    let dir = tempfile::tempdir().unwrap();
    let server = bridge(&dir);
    let registration = server.register().unwrap();
    let mut bad = Socket::connect(endpoint(&registration).strip_prefix("unix:").unwrap()).unwrap();
    bad.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    writeln!(bad, "{}", serde_json::json!({"token":"00".repeat(32)})).unwrap();
    let mut bytes = [0u8; 1];
    assert_eq!(read_uninterrupted(&mut bad, &mut bytes).unwrap(), 0);
    let mut live = Vec::new();
    for _ in 0..4 {
        let (mut socket, mut reader) = authenticated(&registration);
        initialize(&mut socket, &mut reader);
        live.push((socket, reader));
    }
    let (_, mut reader) = authenticated(&registration);
    assert!(bounded_line(&mut reader, 1024).unwrap_or(None).is_none());
    drop(live);
}
#[test]
fn stop_cleans_listener_and_allows_reenable() {
    let dir = tempfile::tempdir().unwrap();
    let server = bridge(&dir);
    let first = server.register().unwrap();
    let (mut socket, mut reader) = authenticated(&first);
    initialize(&mut socket, &mut reader);
    server.stop();
    assert!(!std::path::Path::new(endpoint(&first).strip_prefix("unix:").unwrap()).exists());
    assert!(bounded_line(&mut reader, 1024).unwrap_or(None).is_none());
    let second = server.register().unwrap();
    assert_ne!(endpoint(&first), endpoint(&second));
    assert_ne!(token(&first), token(&second));
    let (mut socket, mut reader) = authenticated(&second);
    initialize(&mut socket, &mut reader);
}
#[test]
fn actual_stdio_relay_roundtrip() {
    #[derive(Clone)]
    struct Output(Arc<Mutex<Vec<u8>>>);
    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let server = bridge(&dir);
    let registration = server.register().unwrap();
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n"
    );
    let bytes = Arc::new(Mutex::new(Vec::new()));
    crate::run_relay(
        io::Cursor::new(input.as_bytes().to_vec()),
        Output(bytes.clone()),
        &endpoint(&registration),
        &token(&registration),
    )
    .unwrap();
    let output = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    let responses: Vec<Value> = output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 2);
    let names: std::collections::BTreeSet<_> = responses[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "list_terminals",
            "open_terminal",
            "read_terminal",
            "send_input",
            "run_command",
            "exec_command",
            "read_command",
            "cancel_command",
        ]
        .into_iter()
        .collect(),
    );
}
#[test]
fn bounds_unauthenticated_connections_and_protocol_lines() {
    let dir = tempfile::tempdir().unwrap();
    let server = bridge(&dir);
    let registration = server.register().unwrap();
    let endpoint = endpoint(&registration);
    let path = endpoint.strip_prefix("unix:").unwrap();
    let mut oversized = Socket::connect(path).unwrap();
    oversized
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    oversized.write_all(&[b'x'; 514]).unwrap();
    let mut byte = [0u8; 1];
    assert!(read_uninterrupted(&mut oversized, &mut byte).unwrap_or(0) == 0);
    let (mut socket, mut reader) = authenticated(&registration);
    initialize(&mut socket, &mut reader);
    let mut bytes = vec![b'x'; nocterm_ai::MAX_MCP_LINE + 1];
    bytes.push(b'\n');
    let _ = socket.write_all(&bytes);
    assert!(bounded_line(&mut reader, 1024).unwrap_or(None).is_none());
    let connections: Vec<_> = (0..64).map(|_| Socket::connect(path).unwrap()).collect();
    for _ in 0..100 {
        if server.inner.connections.load(Ordering::Acquire) == 64 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(server.inner.connections.load(Ordering::Acquire), 64);
    let mut rejected = Socket::connect(path).unwrap();
    rejected
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    assert_eq!(read_uninterrupted(&mut rejected, &mut byte).unwrap(), 0);
    drop(connections);
    server.stop();
    for _ in 0..100 {
        if server.inner.connections.load(Ordering::Acquire) == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(server.inner.connections.load(Ordering::Acquire), 0);
}

#[test]
fn rejected_requests_reach_the_chat_and_keep_the_json_rpc_error() {
    let dir = tempfile::tempdir().unwrap();
    let server = bridge(&dir);
    let registration = server.register().unwrap();
    let (mut socket, mut reader) = authenticated(&registration);
    initialize(&mut socket, &mut reader);
    for (tool, arguments) in [
        ("unknown_tool", serde_json::json!({"argument":"bad"})),
        (
            "exec_command",
            serde_json::json!({"terminal_id":"t1","program":"sh","timeout_ms":1_800_000}),
        ),
        ("exec_command", serde_json::json!({"program":42})),
    ] {
        writeln!(socket,"{}",serde_json::json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":tool,"arguments":arguments}})).unwrap();
        let rejection = server.rejections().recv_blocking().unwrap();
        assert_eq!(rejection.registration_id, registration.id);
        assert_eq!(rejection.tool, tool);
        assert_eq!(rejection.arguments, arguments);
        assert!(!rejection.error.is_empty());
        let response: Value =
            serde_json::from_str(&bounded_line(&mut reader, 1024 * 1024).unwrap().unwrap())
                .unwrap();
        assert_eq!(response["id"], 9);
        assert_eq!(response["error"]["code"], -32602);
        assert!(response.get("result").is_none());
    }
}
