use std::{
    collections::BTreeMap,
    io::{self, BufRead, BufReader, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use futures::{
    channel::oneshot,
    future::{Either, select},
};
use nocterm_ai::{
    BridgeCall, BridgeRegistration, ToolBridge,
    mcp::{McpSession, McpStep},
};
use nocterm_core::Paths;
use serde_json::Value;
use subtle::ConstantTimeEq as _;

#[cfg(unix)]
pub(crate) type Socket = std::os::unix::net::UnixStream;
#[cfg(not(unix))]
pub(crate) type Socket = std::net::TcpStream;
#[cfg(unix)]
type Listener = std::os::unix::net::UnixListener;
#[cfg(not(unix))]
type Listener = std::net::TcpListener;

struct Registration {
    token: String,
    sockets: BTreeMap<u64, Socket>,
    cancel: async_channel::Sender<()>,
    cancelled: async_channel::Receiver<()>,
}
struct Listening {
    endpoint: String,
    path: Option<PathBuf>,
}
#[derive(Default)]
struct State {
    listening: Option<Listening>,
    registrations: BTreeMap<u64, Registration>,
    sockets: BTreeMap<u64, Socket>,
}
struct Inner {
    paths: Paths,
    state: Mutex<State>,
    generation: AtomicU64,
    next_id: AtomicU64,
    connections: AtomicUsize,
    calls_tx: async_channel::Sender<BridgeCall>,
    calls_rx: async_channel::Receiver<BridgeCall>,
}

/// Lazily opens a private per-instance listener; stopping allows later reactivation.
pub struct BridgeServer {
    inner: Arc<Inner>,
}
impl BridgeServer {
    pub fn new(paths: Paths) -> Self {
        let (calls_tx, calls_rx) = async_channel::bounded(256);
        Self {
            inner: Arc::new(Inner {
                paths,
                state: Mutex::new(State::default()),
                generation: AtomicU64::new(0),
                next_id: AtomicU64::new(1),
                connections: AtomicUsize::new(0),
                calls_tx,
                calls_rx,
            }),
        }
    }
}
impl ToolBridge for BridgeServer {
    fn register(&self) -> Result<BridgeRegistration, String> {
        let token = random_token()?;
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.listening.is_none() {
            let (listener, listening) = listen(&self.inner.paths)?;
            listener.set_nonblocking(true).map_err(|e| e.to_string())?;
            let epoch = self.inner.generation.fetch_add(1, Ordering::AcqRel) + 1;
            let inner = self.inner.clone();
            std::thread::Builder::new()
                .name("nocterm-agent-bridge".into())
                .spawn(move || {
                    while inner.generation.load(Ordering::Acquire) == epoch {
                        match listener.accept() {
                            Ok((socket, _)) => {
                                if inner.connections.fetch_add(1, Ordering::AcqRel) >= 64 {
                                    inner.connections.fetch_sub(1, Ordering::AcqRel);
                                    continue;
                                }
                                let inner = inner.clone();
                                let connection_id = inner.next_id.fetch_add(1, Ordering::Relaxed);
                                let count = ConnectionCount(inner.clone(), connection_id);
                                {
                                    let mut state =
                                        inner.state.lock().unwrap_or_else(|e| e.into_inner());
                                    if inner.generation.load(Ordering::Acquire) != epoch {
                                        drop(state);
                                        drop(count);
                                        continue;
                                    }
                                    let Ok(tracked) = socket.try_clone() else {
                                        drop(state);
                                        drop(count);
                                        continue;
                                    };
                                    state.sockets.insert(connection_id, tracked);
                                }
                                let _ = std::thread::Builder::new()
                                    .name("nocterm-bridge-client".into())
                                    .spawn(move || {
                                        let _count = count;
                                        let _ = serve(socket, inner, epoch, connection_id);
                                    });
                            }
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                                std::thread::sleep(Duration::from_millis(20))
                            }
                            Err(_) => break,
                        }
                    }
                })
                .map_err(|e| e.to_string())?;
            state.listening = Some(listening);
        }
        let endpoint = state
            .listening
            .as_ref()
            .expect("listener created")
            .endpoint
            .clone();
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (cancel, cancelled) = async_channel::bounded(1);
        state.registrations.insert(
            id,
            Registration {
                token: token.clone(),
                sockets: BTreeMap::new(),
                cancel,
                cancelled,
            },
        );
        let inner = Arc::downgrade(&self.inner);
        Ok(BridgeRegistration::new(
            id,
            endpoint,
            token,
            Arc::new(move |id| {
                if let Some(inner) = inner.upgrade() {
                    revoke(&inner, id);
                }
            }),
        ))
    }
    fn calls(&self) -> async_channel::Receiver<BridgeCall> {
        self.inner.calls_rx.clone()
    }
    fn stop(&self) {
        self.inner.generation.fetch_add(1, Ordering::AcqRel);
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        for (_, registration) in std::mem::take(&mut state.registrations) {
            close_registration(registration);
        }
        for (_, socket) in std::mem::take(&mut state.sockets) {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
        if let Some(listening) = state.listening.take()
            && let Some(path) = listening.path
        {
            let _ = std::fs::remove_file(path);
        }
        // The runtime may be disabled while requests still await foreground routing.
        while let Ok(call) = self.inner.calls_rx.try_recv() {
            let _ = call.respond.send(Err("AI bridge stopped".into()));
        }
    }
}
impl Drop for BridgeServer {
    fn drop(&mut self) {
        self.stop();
    }
}
struct ConnectionCount(Arc<Inner>, u64);
impl Drop for ConnectionCount {
    fn drop(&mut self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .sockets
            .remove(&self.1);
        self.0.connections.fetch_sub(1, Ordering::AcqRel);
    }
}

fn random_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
fn token_eq(a: &str, b: &str) -> bool {
    if a.len() != 64 || b.len() != 64 {
        return false;
    }
    bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}
fn listen(paths: &Paths) -> Result<(Listener, Listening), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = paths.ensure_runtime_dir().map_err(|e| e.to_string())?;
        let suffix = random_token()?;
        let path = directory.join(format!("a-{}.sock", &suffix[..12]));
        let listener = Listener::bind(&path).map_err(|e| e.to_string())?;
        if let Err(error) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        {
            let _ = std::fs::remove_file(&path);
            return Err(error.to_string());
        }
        Ok((
            listener,
            Listening {
                endpoint: format!("unix:{}", path.display()),
                path: Some(path),
            },
        ))
    }
    #[cfg(not(unix))]
    {
        let _ = paths;
        let listener = Listener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let endpoint = format!("tcp:{}", listener.local_addr().map_err(|e| e.to_string())?);
        Ok((
            listener,
            Listening {
                endpoint,
                path: None,
            },
        ))
    }
}
fn close_registration(registration: Registration) {
    // Disconnect before cancelling: a call woken by the cancel must find its
    // socket already shut, not slip a reply in ahead of the disconnect.
    for (_, socket) in registration.sockets {
        let _ = socket.shutdown(std::net::Shutdown::Both);
    }
    registration.cancel.close();
}
fn revoke(inner: &Inner, id: u64) {
    let registration = inner
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .registrations
        .remove(&id);
    if let Some(registration) = registration {
        close_registration(registration);
    }
}

pub(crate) fn bounded_line<R: BufRead>(reader: &mut R, limit: usize) -> io::Result<Option<String>> {
    let mut bytes = Vec::new();
    loop {
        // A signal handled elsewhere in the process interrupts a socket read
        // that has a timeout even under SA_RESTART; the line is still coming.
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if available.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Incomplete protocol line",
                ))
            };
        }
        let end = available.iter().position(|&b| b == b'\n');
        let count = end.map_or(available.len(), |end| end + 1);
        if bytes.len() + count > limit + 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Protocol line too large",
            ));
        }
        bytes.extend_from_slice(&available[..count]);
        reader.consume(count);
        if end.is_some() {
            bytes.pop();
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
            return String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid UTF-8"));
        }
    }
}

fn serve(socket: Socket, inner: Arc<Inner>, epoch: u64, connection_id: u64) -> io::Result<()> {
    socket.set_read_timeout(Some(Duration::from_secs(5)))?;
    socket.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(socket.try_clone()?);
    let hello = bounded_line(&mut reader, 512)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Missing bridge authentication",
        )
    })?;
    let hello: Value = serde_json::from_str(&hello).map_err(|_| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Invalid bridge authentication",
        )
    })?;
    let token = hello
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let (id, cancel) = {
        let mut state = inner.state.lock().unwrap_or_else(|e| e.into_inner());
        if inner.generation.load(Ordering::Acquire) != epoch {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Bridge stopped",
            ));
        }
        let total: usize = state.registrations.values().map(|v| v.sockets.len()).sum();
        if total >= 64 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Bridge connection limit",
            ));
        }
        let (id, registration) = state
            .registrations
            .iter_mut()
            .find(|(_, r)| token_eq(token, &r.token))
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::PermissionDenied, "Invalid bridge token")
            })?;
        if registration.sockets.len() >= 4 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Thread connection limit",
            ));
        }
        registration
            .sockets
            .insert(connection_id, socket.try_clone()?);
        (*id, registration.cancelled.clone())
    };
    let result = serve_authenticated(socket, &mut reader, &inner, id, cancel);
    let mut state = inner.state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(registration) = state.registrations.get_mut(&id) {
        registration.sockets.remove(&connection_id);
    }
    result
}
fn serve_authenticated(
    mut socket: Socket,
    reader: &mut BufReader<Socket>,
    inner: &Inner,
    id: u64,
    cancel: async_channel::Receiver<()>,
) -> io::Result<()> {
    socket.set_read_timeout(None)?;
    let mut session = McpSession::default();
    while let Some(line) = bounded_line(reader, nocterm_ai::MAX_MCP_LINE)? {
        if cancel.is_closed() {
            break;
        }
        let response = match session.handle(&line) {
            McpStep::Reply(reply) => reply,
            McpStep::Ignore => continue,
            McpStep::Call {
                id: request_id,
                call,
            } => {
                let (respond, answer) = oneshot::channel();
                let queued = {
                    let state = inner.state.lock().unwrap_or_else(|e| e.into_inner());
                    state.registrations.contains_key(&id)
                        && inner
                            .calls_tx
                            .try_send(BridgeCall {
                                registration_id: id,
                                call,
                                respond,
                            })
                            .is_ok()
                };
                let result = if !queued {
                    Err("Bridge is closed or busy".into())
                } else {
                    futures::executor::block_on(async {
                        match select(answer, Box::pin(cancel.recv())).await {
                            Either::Left((Ok(reply), _)) => reply,
                            _ => Err("Thread closed".into()),
                        }
                    })
                };
                nocterm_ai::mcp::tool_result(request_id, result)
            }
        };
        let bytes = serde_json::to_vec(&response)?;
        if bytes.len() > nocterm_ai::MAX_MCP_LINE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Bridge response too large",
            ));
        }
        socket.write_all(&bytes)?;
        socket.write_all(b"\n")?;
        socket.flush()?;
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
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
    fn authenticated(registration: &BridgeRegistration) -> (Socket, BufReader<Socket>) {
        let mut socket =
            Socket::connect(registration.endpoint.strip_prefix("unix:").unwrap()).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        writeln!(
            socket,
            "{}",
            serde_json::json!({"token":registration.token})
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
        let server = BridgeServer::new(Paths::rooted_at(dir.path()));
        let mut registration = server.register().unwrap();
        let path = registration.endpoint.strip_prefix("unix:").unwrap();
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
            serde_json::from_str(&bounded_line(&mut reader, 1024 * 1024).unwrap().unwrap())
                .unwrap();
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
        let server = BridgeServer::new(Paths::rooted_at(dir.path()));
        let registration = server.register().unwrap();
        let mut bad =
            Socket::connect(registration.endpoint.strip_prefix("unix:").unwrap()).unwrap();
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
        let server = BridgeServer::new(Paths::rooted_at(dir.path()));
        let first = server.register().unwrap();
        let (mut socket, mut reader) = authenticated(&first);
        initialize(&mut socket, &mut reader);
        server.stop();
        assert!(!std::path::Path::new(first.endpoint.strip_prefix("unix:").unwrap()).exists());
        assert!(bounded_line(&mut reader, 1024).unwrap_or(None).is_none());
        let second = server.register().unwrap();
        assert_ne!(first.endpoint, second.endpoint);
        assert_ne!(first.token, second.token);
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
        let server = BridgeServer::new(Paths::rooted_at(dir.path()));
        let registration = server.register().unwrap();
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n"
        );
        let bytes = Arc::new(Mutex::new(Vec::new()));
        crate::run_relay(
            io::Cursor::new(input.as_bytes().to_vec()),
            Output(bytes.clone()),
            &registration.endpoint,
            &registration.token,
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
        let server = BridgeServer::new(Paths::rooted_at(dir.path()));
        let registration = server.register().unwrap();
        let path = registration.endpoint.strip_prefix("unix:").unwrap();
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
}
