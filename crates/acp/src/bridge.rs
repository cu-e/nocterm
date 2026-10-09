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
    BridgeCall, BridgeLaunch, BridgeRegistration, BridgeRejection, ToolBridge,
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
/// The command an agent runs to reach the bridge. It must call
/// [`crate::run_relay_from_environment`].
#[derive(Clone, Debug)]
pub struct RelayCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
}

pub(crate) const ENDPOINT_VARIABLE: &str = "NOCTERM_BRIDGE_ENDPOINT";
pub(crate) const TOKEN_VARIABLE: &str = "NOCTERM_BRIDGE_TOKEN";

struct Inner {
    paths: Paths,
    relay: RelayCommand,
    state: Mutex<State>,
    generation: AtomicU64,
    next_id: AtomicU64,
    connections: AtomicUsize,
    calls_tx: async_channel::Sender<BridgeCall>,
    calls_rx: async_channel::Receiver<BridgeCall>,
    rejections_tx: async_channel::Sender<BridgeRejection>,
    rejections_rx: async_channel::Receiver<BridgeRejection>,
}

/// Lazily opens a private per-instance listener; stopping allows later reactivation.
pub struct BridgeServer {
    inner: Arc<Inner>,
}
impl BridgeServer {
    pub fn new(paths: Paths, relay: RelayCommand) -> Self {
        let (calls_tx, calls_rx) = async_channel::bounded(256);
        let (rejections_tx, rejections_rx) = async_channel::bounded(256);
        Self {
            inner: Arc::new(Inner {
                paths,
                relay,
                state: Mutex::new(State::default()),
                generation: AtomicU64::new(0),
                next_id: AtomicU64::new(1),
                connections: AtomicUsize::new(0),
                calls_tx,
                calls_rx,
                rejections_tx,
                rejections_rx,
            }),
        }
    }
}
impl ToolBridge for BridgeServer {
    fn register(&self) -> Result<BridgeRegistration, String> {
        let relay = &self.inner.relay;
        if !relay.program.is_file() {
            // The agent would start without terminal tools and only say so in chat.
            return Err(format!(
                "Terminal tools need {}, which no longer exists. Restart nocterm.",
                relay.program.display()
            ));
        }
        let token = random_token()?;
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let endpoint = start_listening(&self.inner, &mut state)?;
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
        let launch = BridgeLaunch {
            program: relay.program.clone(),
            args: relay.args.clone(),
            env: vec![
                (ENDPOINT_VARIABLE.into(), endpoint),
                (TOKEN_VARIABLE.into(), token),
            ],
        };
        Ok(BridgeRegistration::new(
            id,
            launch,
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
    fn rejections(&self) -> async_channel::Receiver<BridgeRejection> {
        self.inner.rejections_rx.clone()
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
/// Opens the listener on first use and returns the endpoint relays connect to.
fn start_listening(inner: &Arc<Inner>, state: &mut State) -> Result<String, String> {
    if state.listening.is_none() {
        let (listener, listening) = listen(&inner.paths)?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let epoch = inner.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let inner = inner.clone();
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
    Ok(state
        .listening
        .as_ref()
        .expect("listener created")
        .endpoint
        .clone())
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
            McpStep::Rejected {
                id: request_id,
                tool,
                arguments,
                error,
            } => {
                let _ = inner.rejections_tx.try_send(BridgeRejection {
                    registration_id: id,
                    tool,
                    arguments,
                    error: error.clone(),
                });
                serde_json::json!({"jsonrpc":"2.0", "id":request_id,
                    "error":{"code":-32602,"message":error}})
            }
            McpStep::Call {
                id: request_id,
                call,
                arguments,
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
                                arguments: Some(arguments),
                                display_token: None,
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
mod tests;
