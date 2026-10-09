mod client;
mod documents;
mod lease;
mod lifecycle;
mod settings;
pub use client::{Client, ClientState, SessionClient, SessionEvent};
use gpui::{App, Context, Entity, EntityId, Global, Task};
pub use lease::SessionLease;
use nocterm_ai::{
    AgentCommands, AgentConnector, AgentEvent, AgentInfo, AgentLaunch, BridgeRegistration,
    ConnectRequest, ToolBridge, acp,
};
use settings::AiSettingsExt as _;
pub use settings::AiSettingsSource;
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::Arc,
};

pub struct TerminalAuthRequest {
    pub program: String,
    pub args: Vec<String>,
    pub env: std::collections::BTreeMap<String, String>,
    pub cwd: PathBuf,
    pub title: String,
}
/// What the runtime connects agents with, and where it keeps their state.
pub struct RuntimeServices {
    pub connector: Arc<dyn AgentConnector>,
    pub bridge: Arc<dyn ToolBridge>,
    pub state_file: PathBuf,
    /// Saved chats, one file each.
    pub chats_dir: PathBuf,
    /// Where Codex keeps its session logs, read for its plan limits.
    pub codex_home: Option<PathBuf>,
    pub workdir: PathBuf,
    /// Whether the host can run an agent's sign-in command in a terminal.
    pub terminal_auth: bool,
    /// nocterm's own directories (settings, vault, chats), hidden from
    /// isolated agents.
    pub private_dirs: Vec<PathBuf>,
    /// Directories isolated agents must still reach, such as the terminal
    /// tools' socket directory.
    pub shared_dirs: Vec<PathBuf>,
}
pub struct RuntimeGlobal(pub Entity<Runtime>);
impl Global for RuntimeGlobal {}
struct Connection {
    chat_id: String,
    launch: AgentLaunch,
    workdir: PathBuf,
    /// Whether the process runs isolated; a changed setting restarts it.
    isolated: bool,
    commands: Option<Arc<dyn AgentCommands>>,
    info: Option<AgentInfo>,
    users: HashMap<EntityId, Client>,
    serial: u64,
    _events: Option<Task<()>>,
    _connecting: Option<Task<()>>,
    cancellation: nocterm_ai::ConnectionCancellation,
    startup_completion: Option<futures::channel::oneshot::Receiver<Result<(), String>>>,
}
pub struct Runtime {
    pub services: RuntimeServices,
    pub favorites: nocterm_ai::favorites::AgentStateFile,
    pub favorites_error: Option<String>,
    favorites_revision: u64,
    favorites_writer: Option<Task<()>>,
    connections: HashMap<u64, Connection>,
    documents: HashMap<EntityId, Client>,
    document_owners: HashMap<String, EntityId>,
    pending_activation: std::collections::VecDeque<Client>,
    closing: HashMap<u64, String>,
    idle_since: HashMap<u64, std::time::Instant>,
    _lifecycle: Option<Task<()>>,
    shutting_down: bool,
    chat_revisions: HashMap<String, (EntityId, u64)>,
    pub registrations: HashMap<u64, Client>,
    /// Chats read from disk and not yet shown by a panel; `None` while reading.
    pub saved_chats: Option<Vec<nocterm_ai::history::SavedChat>>,
    /// Latest unsaved snapshot of each chat, written in order by `chat_writer`.
    chat_writes: HashMap<String, Option<Arc<nocterm_ai::history::SharedChat>>>,
    chat_writer: Option<Task<()>>,
    chat_in_flight: Option<documents::PendingChatWrite>,
    chat_io: Arc<std::sync::Mutex<documents::ChatIoGate>>,
    /// Deleted chats, never written again by a late save.
    deleted_chats: std::collections::HashSet<String>,
    _loading_chats: Option<Task<()>>,
    /// Plan limits last reported, by agent.
    limits: HashMap<String, nocterm_ai::usage::Limits>,
    /// Agents whose limits are being read.
    reading_limits: std::collections::HashSet<String>,
    serial: u64,
    /// Leases dropped by their threads, waiting to be released.
    lease_releases: async_channel::Sender<lease::ReleasedLease>,
    _releasing: Task<()>,
    _bridge: Option<Task<()>>,
}
impl Runtime {
    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<RuntimeGlobal>().0.clone()
    }
    pub fn new(services: RuntimeServices, cx: &mut Context<Self>) -> Self {
        for warning in nocterm_ai::AgentRegistry::new(cx.ai()).warnings {
            tracing::warn!(message=%nocterm_ai::redact::redact(&warning),"Ignoring AI agent configuration");
        }
        let favorites =
            nocterm_ai::favorites::AgentStateFile::load(&services.state_file).unwrap_or_default();
        let chats_dir = services.chats_dir.clone();
        let loading = cx
            .background_executor()
            .spawn(async move { nocterm_ai::history::load_all(&chats_dir) });
        let loading_chats = cx.spawn(async move |this, cx| {
            let chats = loading.await;
            let _ = this.update(cx, |this, cx| {
                this.saved_chats = Some(chats);
                cx.notify();
            });
        });
        let (lease_releases, released) = async_channel::unbounded();
        let releasing = cx.spawn(async move |this, cx| {
            while let Ok(lease) = released.recv().await {
                if this
                    .update(cx, |this, cx| this.release_lease(lease, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            saved_chats: None,
            chat_writes: HashMap::new(),
            chat_writer: None,
            chat_in_flight: None,
            chat_io: Default::default(),
            deleted_chats: Default::default(),
            _loading_chats: Some(loading_chats),
            limits: HashMap::new(),
            reading_limits: Default::default(),
            favorites,
            favorites_error: None,
            favorites_revision: 0,
            favorites_writer: None,
            services,
            connections: HashMap::new(),
            documents: HashMap::new(),
            document_owners: HashMap::new(),
            pending_activation: Default::default(),
            closing: Default::default(),
            idle_since: Default::default(),
            _lifecycle: None,
            shutting_down: false,
            chat_revisions: Default::default(),
            registrations: HashMap::new(),
            serial: 0,
            lease_releases,
            _releasing: releasing,
            _bridge: None,
        }
    }
    /// Applies changed AI settings to the running agents.
    pub fn settings_changed(&mut self, cx: &mut Context<Self>) {
        if !cx.ai_enabled() {
            let ids = self.documents.keys().copied().collect::<Vec<_>>();
            for id in ids {
                self.capture_and_detach_document(id, cx);
            }
        }
        let registry = nocterm_ai::AgentRegistry::new(cx.ai());
        for warning in &registry.warnings {
            tracing::warn!(message=%nocterm_ai::redact::redact(warning),"Ignoring AI agent configuration");
        }
        let keys: Vec<_> = self
            .connections
            .iter()
            .filter(|(_, connection)| registry.get(&connection.launch.id).is_none())
            .map(|(key, _)| *key)
            .collect();
        for key in keys {
            self.stop_connection(key, "Agent configuration changed. Start a new chat.", cx);
        }
        self.apply_isolation(cx);
        if !cx.ai_enabled() {
            self.registrations.clear();
            self.services.bridge.stop();
        }
        let clients: Vec<_> = self
            .connections
            .values()
            .flat_map(|connection| connection.users.values().cloned())
            .collect();
        for client in clients {
            client.emit(SessionEvent::PolicyChanged, cx);
        }
    }
    /// Isolation is a security boundary: it applies to running agents at once
    /// rather than to the next chat.
    fn apply_isolation(&mut self, cx: &mut Context<Self>) {
        let isolated = cx.ai().sandbox == nocterm_ai::SandboxMode::Workspace;
        let keys: Vec<_> = self
            .connections
            .iter()
            .filter(|(_, connection)| connection.isolated != isolated)
            .map(|(key, _)| *key)
            .collect();
        for key in keys {
            self.stop_connection(
                key,
                if isolated {
                    "Agent isolation was turned on. Restart the chat to continue isolated."
                } else {
                    "Agent isolation was turned off. Restart the chat to continue."
                },
                cx,
            );
        }
    }
    pub fn toggle_favorite(
        &mut self,
        agent: &str,
        option: &str,
        value: &str,
        cx: &mut Context<Self>,
    ) {
        self.favorites.toggle(agent, option, value);
        self.save_state(cx);
    }
    /// Remembers `agent` as the one new chats start with by default.
    pub fn set_last_agent(&mut self, agent: &str, cx: &mut Context<Self>) {
        if self.favorites.last_agent.as_deref() == Some(agent) {
            return;
        }
        self.favorites.last_agent = Some(agent.to_owned());
        self.save_state(cx);
    }
    fn save_state(&mut self, cx: &mut Context<Self>) {
        self.favorites_revision += 1;
        cx.notify();
        if self.favorites_writer.is_some() {
            return;
        }
        self.favorites_writer = Some(cx.spawn(async move |this, cx| {
            loop {
                let Ok((state, path, revision)) = this.read_with(cx, |this, _| {
                    (
                        this.favorites.clone(),
                        this.services.state_file.clone(),
                        this.favorites_revision,
                    )
                }) else {
                    return;
                };
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        if let Some(parent) = path.parent() {
                            nocterm_core::paths::ensure_private_dir(parent)
                                .map_err(|error| error.to_string())?;
                        }
                        state.save(&path)
                    })
                    .await;
                let done = this
                    .update(cx, |this, cx| {
                        this.favorites_error = result.err();
                        cx.notify();
                        if this.favorites_revision == revision {
                            this.favorites_writer.take();
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(true);
                if done {
                    return;
                }
            }
        }));
    }
    /// The plan limits last reported for `agent`.
    pub fn limits(&self, agent: &str) -> Option<&nocterm_ai::usage::Limits> {
        self.limits.get(agent).filter(|limits| !limits.is_empty())
    }
    /// Reads `agent`'s plan limits again, from agents that keep them in their
    /// own files. Others report them along with usage.
    pub fn refresh_limits(&mut self, agent: &str, cx: &mut Context<Self>) {
        let codex = nocterm_ai::AgentRegistry::new(cx.ai())
            .get(agent)
            .is_some_and(nocterm_ai::usage::is_codex);
        let Some(home) = self.services.codex_home.clone().filter(|_| codex) else {
            return;
        };
        if !self.reading_limits.insert(agent.to_owned()) {
            return;
        }
        let agent = agent.to_owned();
        let reading = cx
            .background_executor()
            .spawn(async move { nocterm_ai::usage::read_codex_limits(&home) });
        cx.spawn(async move |this, cx| {
            let limits = reading.await;
            let _ = this.update(cx, |this, cx| {
                this.reading_limits.remove(&agent);
                if let Some(limits) = limits {
                    this.limits.entry(agent).or_default().replace(limits);
                    cx.notify();
                }
            });
        })
        .detach();
    }
    pub fn start_bridge(&mut self, cx: &mut Context<Self>) {
        if self._bridge.is_some() {
            return;
        }
        let calls = self.services.bridge.calls();
        let rejections = self.services.bridge.rejections();
        self._bridge = Some(cx.spawn(async move |this, cx| {
            loop {
                use futures::future::{Either, select};
                let event = if rejections.is_closed() && rejections.is_empty() {
                    match calls.recv().await {
                        Ok(call) => SessionEvent::Tool(call),
                        Err(_) => break,
                    }
                } else {
                    match select(Box::pin(calls.recv()), Box::pin(rejections.recv())).await {
                        Either::Left((Ok(call), _)) => SessionEvent::Tool(call),
                        Either::Right((Ok(rejected), _)) => SessionEvent::ToolRejected(rejected),
                        Either::Left((Err(_), _)) => break,
                        Either::Right((Err(_), _)) => continue,
                    }
                };
                let registration = match &event {
                    SessionEvent::Tool(call) => call.registration_id,
                    SessionEvent::ToolRejected(rejected) => rejected.registration_id,
                    _ => unreachable!(),
                };
                let client = this
                    .read_with(cx, |this, _| {
                        this.registrations
                            .get(&registration)
                            .filter(|client| client.alive())
                            .cloned()
                    })
                    .ok()
                    .flatten();
                if let Some(client) = client {
                    cx.update(|cx| client.emit(event, cx));
                } else if let SessionEvent::Tool(call) = event {
                    let _ = call
                        .respond
                        .send(Err("Chat was closed or AI is disabled.".into()));
                }
            }
        }));
    }

    pub fn register_bridge(
        &mut self,
        client: &Client,
        cx: &mut Context<Self>,
    ) -> Result<BridgeRegistration, String> {
        if !cx.ai_enabled() {
            return Err("AI is disabled.".into());
        }
        let registration = self.services.bridge.register()?;
        self.registrations.insert(registration.id, client.clone());
        self.start_bridge(cx);
        Ok(registration)
    }
    pub fn terminal_auth_request(
        &self,
        key: u64,
        method: &acp::AuthMethodTerminal,
    ) -> Option<TerminalAuthRequest> {
        let connection = self.connections.get(&key)?;
        let launch = &connection.launch;
        let mut args = launch.args.clone();
        args.extend(method.args.iter().cloned());
        let mut env = nocterm_ai::env::sanitized_environment(launch, std::env::vars());
        env.extend(
            method
                .env
                .iter()
                .filter(|(key, value)| {
                    nocterm_ai::env::valid_name(key)
                        && !nocterm_ai::env::blocked(key)
                        && !value.contains('\0')
                })
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        Some(TerminalAuthRequest {
            program: launch.command.clone(),
            args,
            env,
            cwd: connection.workdir.clone(),
            title: format!("Sign in: {}", method.name),
        })
    }
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    pub fn connect(
        &mut self,
        client: Client,
        chat_id: String,
        launch: AgentLaunch,
        cx: &mut Context<Self>,
    ) {
        if !cx.ai_enabled() {
            return;
        }
        let workdir = cx
            .ai()
            .working_directory
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.services.workdir.clone());
        let isolated = cx.ai().sandbox == nocterm_ai::SandboxMode::Workspace;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        launch.hash(&mut hasher);
        workdir.hash(&mut hasher);
        isolated.hash(&mut hasher);
        self.serial += 1;
        self.serial.hash(&mut hasher);
        let key = hasher.finish();
        let registration = match self.register_bridge(&client, cx) {
            Ok(registration) => registration,
            Err(error) => {
                client.emit(SessionEvent::Failed(error), cx);
                return;
            }
        };
        let lease = SessionLease::new(client.id(), key, registration, self.lease_releases.clone());
        client.emit(SessionEvent::Leased(lease), cx);
        self.serial += 1;
        let serial = self.serial;
        let cancellation = nocterm_ai::ConnectionCancellation::default();
        let (startup_ack, startup_completion) = futures::channel::oneshot::channel();
        self.connections.insert(
            key,
            Connection {
                chat_id,
                launch: launch.clone(),
                workdir: workdir.clone(),
                isolated,
                commands: None,
                info: None,
                users: HashMap::from([(client.id(), client.clone())]),
                serial,
                _events: None,
                _connecting: None,
                cancellation: cancellation.clone(),
                startup_completion: Some(startup_completion),
            },
        );
        let connector = self.services.connector.clone();
        let private_workdir = workdir == self.services.workdir;
        let terminal_auth = self.services.terminal_auth;
        let sandbox = isolated.then(|| {
            nocterm_ai::sandbox::SandboxPolicy::new(
                &workdir,
                std::env::home_dir().as_deref(),
                &self.services.private_dirs,
                &self.services.shared_dirs,
            )
        });
        let resources = cx.ai().resources.clone();
        let future = cx.background_executor().spawn(async move {
            if private_workdir {
                nocterm_core::paths::ensure_private_dir(&workdir)
                    .map_err(|error| nocterm_ai::AgentError::Io(error.to_string()))?;
            } else if !workdir.is_absolute() || !workdir.is_dir() {
                return Err(nocterm_ai::AgentError::Io(
                    "Working directory must be an existing absolute directory.".into(),
                ));
            }
            connector
                .connect(ConnectRequest {
                    launch,
                    working_directory: workdir,
                    terminal_auth,
                    sandbox,
                    resources,
                    cancellation,
                })
                .await
        });
        let connecting = cx.spawn(async move |this, cx| {
            let result = future.await;
            let uncertain = match &result {
                Err(nocterm_ai::AgentError::CleanupUnconfirmed(error)) => Some(error.clone()),
                _ => None,
            };
            let orphan = result
                .as_ref()
                .ok()
                .map(|connection| connection.commands.clone());
            let accepted = this
                .update(cx, |this, cx| {
                    if !this
                        .connections
                        .get(&key)
                        .is_some_and(|connection| connection.serial == serial)
                        || !cx.ai_enabled()
                    {
                        return false;
                    }
                    match result {
                        Ok(connection) => {
                            let slot = this.connections.get_mut(&key).expect("checked connection");
                            slot.commands = Some(connection.commands.clone());
                            slot.info = Some(connection.info.clone());
                            let workdir = slot.workdir.clone();
                            let clients: Vec<_> = slot.users.values().cloned().collect();
                            for client in clients {
                                client.emit(
                                    SessionEvent::Connected {
                                        commands: connection.commands.clone(),
                                        info: Box::new(connection.info.clone()),
                                        workdir: workdir.clone(),
                                    },
                                    cx,
                                );
                            }
                            slot._events = Some(cx.spawn(async move |this, cx| {
                                while let Ok(event) = connection.events.recv().await {
                                    if this
                                        .update(cx, |this, cx| this.event(key, serial, event, cx))
                                        .is_err()
                                    {
                                        break;
                                    }
                                }
                            }));
                        }
                        Err(error) => this.stop_connection(
                            key,
                            &nocterm_ai::redact::redact(&error.to_string()),
                            cx,
                        ),
                    }
                    true
                })
                .unwrap_or(false);
            let cleanup = if let Some(error) = uncertain {
                Err(error)
            } else if !accepted && let Some(commands) = orphan {
                commands
                    .shutdown_gracefully()
                    .await
                    .map_err(|error| error.to_string())
            } else {
                Ok(())
            };
            let _ = startup_ack.send(cleanup);
        });
        if let Some(connection) = self.connections.get_mut(&key) {
            connection._connecting = Some(connecting);
        }
    }
    fn event(&mut self, key: u64, serial: u64, event: AgentEvent, cx: &mut Context<Self>) {
        if let AgentEvent::Barrier(ack) = event {
            let _ = ack.send(());
            return;
        }
        let Some(connection) = self
            .connections
            .get(&key)
            .filter(|connection| connection.serial == serial)
        else {
            return;
        };
        match event {
            AgentEvent::Barrier(_) => unreachable!("handled before connection routing"),
            AgentEvent::Session(notification) => {
                if let acp::SessionUpdate::UsageUpdate(update) = &notification.update
                    && let Some(limit) = nocterm_ai::usage::claude_limit(update)
                {
                    let agent = connection.launch.id.clone();
                    self.limits.entry(agent).or_default().merge(limit);
                    cx.notify();
                }
                let Some(connection) = self.connections.get(&key) else {
                    return;
                };
                let notification = std::rc::Rc::new(notification);
                let clients: Vec<_> = connection.users.values().cloned().collect();
                for client in clients {
                    client.emit(SessionEvent::Update(notification.clone()), cx);
                }
            }
            AgentEvent::Permission { request, respond } => {
                let client = connection
                    .users
                    .values()
                    .find(|client| {
                        client.state(cx).is_some_and(|state| {
                            state.session.as_ref() == Some(&request.session_id)
                        })
                    })
                    .cloned();
                // A request no thread takes is answered `Cancelled` on drop.
                if let Some(client) = client {
                    client.emit(
                        SessionEvent::Permission {
                            request: Box::new(request),
                            respond,
                        },
                        cx,
                    );
                }
            }
            AgentEvent::Exited { code, stderr_tail } => self.stop_connection(
                key,
                &format!(
                    "Agent exited ({code:?}). {}",
                    nocterm_ai::redact::redact(&stderr_tail)
                ),
                cx,
            ),
        }
    }
    fn stop_connection(&mut self, key: u64, message: &str, cx: &mut Context<Self>) {
        let Some(connection) = self.connections.get(&key) else {
            return;
        };
        connection.cancellation.cancel();
        let clients: Vec<_> = connection.users.values().cloned().collect();
        for client in clients {
            client.emit(SessionEvent::Stopped(message.to_owned()), cx);
        }
    }
    pub fn shutdown(&mut self, cx: &mut Context<Self>) -> Task<()> {
        self.shutting_down = true;
        self.pending_activation.clear();
        let clients: Vec<_> = self.documents.values().cloned().collect();
        for client in clients {
            client.emit(SessionEvent::Shutdown, cx);
        }
        // GPUI allows quit futures only 200 ms. Durable state must be flushed before returning.
        self.flush_chats(cx);
        self.chat_writer.take();
        self.registrations.clear();
        self.services.bridge.stop();
        cx.spawn(async move |this, cx| {
            for _ in 0..100 {
                if this
                    .read_with(cx, |this, _| {
                        this.closing.is_empty() && this.connections.is_empty()
                    })
                    .unwrap_or(true)
                {
                    return;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
            }
            tracing::warn!("Agent cleanup did not complete before application exit");
        })
    }
}
