//! Agent lifecycle and durable chat orchestration.
//!
//! Owns: pure admission policy, connection/lease owners, document identity, bounded
//! keyed write admission, lazy history hydration and usage projections.
//! Does not own: settings storage, history/favorites files, ACP subprocesses or views.
//! Ports used: AgentConnector, ToolBridge, AgentStateRepository, ChatRepository,
//! explicit SettingsHandle and host-supplied ShutdownCoordinator at installation.
//! Ports provided: SessionClient/SessionEvent, HistoryReader and synchronous quit capture.
//! Settings section: ai consumer. Globals: existing RuntimeGlobal compatibility shell.
//! Invariants: one current document owner per ID; delete seals an ID; persistence
//! events refer to the physically published owner/revision; final work reads no App.

mod admission;
mod bridge;
mod catalog;
mod client;
mod releases;
mod usage;
pub use catalog::PreparedHistory;
mod connections;
mod documents;
mod lease;
mod lifecycle;
mod process_cleanup;
mod startup;
mod state;
use futures::future::BoxFuture;
mod storage;
pub use client::{Client, ClientState, SessionClient, SessionEvent};
use gpui::{App, Context, Entity, EntityId, Global, Task};
pub use lease::SessionLease;
use nocterm_ai::{
    AgentCommands, AgentConnector, AgentEvent, AgentInfo, AgentLaunch, BridgeRegistration,
    ToolBridge, acp,
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
pub use storage::{AgentPersistenceError, AgentStorage, HistoryReader};

pub struct TerminalAuthRequest {
    pub program: String,
    pub args: Vec<String>,
    pub env: std::collections::BTreeMap<String, String>,
    pub cwd: PathBuf,
    pub title: String,
}
/// What the runtime connects agents with, and where it keeps their state.
pub struct RuntimeServices {
    pub settings: nocterm_store::SettingsHandle,
    pub connector: Arc<dyn AgentConnector>,
    pub bridge: Arc<dyn ToolBridge>,
    pub storage: AgentStorage,
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
    startup_completion: Option<startup::Completion>,
}
pub struct Runtime {
    services: RuntimeServices,
    state: state::AgentStateStore,
    chats: documents::ChatStore,
    connections: connections::ConnectionManager,
    admission: admission::AdmissionState,
    shutting_down: bool,
    usage: usage::UsageLimits,
    releases: releases::LeaseInbox,
    bridge: bridge::BridgeRegistry,
}
impl Runtime {
    fn ai<'a>(&self, cx: &'a App) -> &'a nocterm_ai::AiSettings {
        self.services.settings.setting::<nocterm_ai::AiSettings>(cx)
    }
    fn ai_enabled(&self, cx: &App) -> bool {
        self.ai(cx).enabled
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<RuntimeGlobal>().0.clone()
    }
    pub fn new(mut services: RuntimeServices, cx: &mut Context<Self>) -> Self {
        for warning in
            nocterm_ai::AgentRegistry::new(services.settings.setting::<nocterm_ai::AiSettings>(cx))
                .warnings
        {
            tracing::warn!(message=%nocterm_ai::redact::redact(&warning),"Ignoring AI agent configuration");
        }
        let state = state::AgentStateStore::new(&services.storage, cx);
        let chats = documents::ChatStore::new(&mut services.storage, cx);
        let releases = releases::LeaseInbox::new(cx);
        Self {
            state,
            chats,
            usage: Default::default(),
            services,
            connections: Default::default(),
            admission: Default::default(),
            shutting_down: false,
            releases,
            bridge: Default::default(),
        }
    }
    /// Applies changed AI settings to the running agents.
    pub fn settings_changed(&mut self, cx: &mut Context<Self>) {
        if self.shutting_down {
            return;
        }
        if !self.ai_enabled(cx) {
            let ids = self
                .chats
                .clients()
                .iter()
                .map(|client| client.id())
                .collect::<Vec<_>>();
            for id in ids {
                self.capture_and_detach_document(id, cx);
            }
        }
        let registry = nocterm_ai::AgentRegistry::new(self.ai(cx));
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
        if !self.ai_enabled(cx) {
            self.bridge.stop();
            self.services.bridge.stop();
        }
        let clients: Vec<_> = self
            .connections
            .iter()
            .map(|(_, connection)| connection)
            .flat_map(|connection| connection.users.values().cloned())
            .collect();
        for client in clients {
            client.emit(SessionEvent::PolicyChanged, cx);
        }
    }
    /// Isolation is a security boundary: it applies to running agents at once
    /// rather than to the next chat.
    fn apply_isolation(&mut self, cx: &mut Context<Self>) {
        let isolated = self.ai(cx).sandbox == nocterm_ai::SandboxMode::Workspace;
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
        if self.shutting_down {
            return;
        }
        self.state.edit(
            |state| {
                state.toggle(agent, option, value);
            },
            cx,
        );
    }
    /// Keeps the configuration options `agent` reported, for chats that
    /// have not connected yet.
    pub fn remember_options(
        &mut self,
        agent: &str,
        options: &[nocterm_ai::acp::SessionConfigOption],
        cx: &mut Context<Self>,
    ) {
        if self.shutting_down {
            return;
        }
        self.state.edit(
            |state| {
                state.remember_options(agent, options);
            },
            cx,
        );
    }
    /// Keeps a model or reasoning effort choice for `agent`'s next chats.
    pub fn remember_choice(
        &mut self,
        agent: &str,
        option: &nocterm_ai::acp::SessionConfigOption,
        value: &nocterm_ai::session_config::ConfigValue,
        cx: &mut Context<Self>,
    ) {
        if self.shutting_down {
            return;
        }
        self.state.edit(
            |state| {
                state.remember_choice(agent, option, value);
            },
            cx,
        );
    }
    /// Remembers `agent` as the one new chats start with by default.
    pub fn set_last_agent(&mut self, agent: &str, cx: &mut Context<Self>) {
        if self.shutting_down {
            return;
        }
        self.state
            .edit(|state| state.last_agent = Some(agent.to_owned()), cx);
    }
    pub fn favorites<'a>(&'a self, cx: &'a App) -> &'a nocterm_ai::favorites::AgentStateFile {
        self.state.visible(cx)
    }
    pub fn favorites_error(&self, cx: &App) -> Option<AgentPersistenceError> {
        self.state.error(cx)
    }
    pub fn history_reader(&self) -> HistoryReader {
        self.chats.reader()
    }
    /// The plan limits last reported for `agent`.
    pub fn limits(&self, agent: &str) -> Option<&nocterm_ai::usage::Limits> {
        self.usage.get(agent)
    }
    /// Reads `agent`'s plan limits again, from agents that keep them in their
    /// own files. Others report them along with usage.
    pub fn refresh_limits(&mut self, agent: &str, cx: &mut Context<Self>) {
        if self.shutting_down {
            return;
        }
        let codex = nocterm_ai::AgentRegistry::new(self.ai(cx))
            .get(agent)
            .is_some_and(nocterm_ai::usage::is_codex);
        let Some(home) = self.services.codex_home.clone().filter(|_| codex) else {
            return;
        };
        self.usage.refresh(agent.to_owned(), home, cx);
    }
    pub fn start_bridge(&mut self, cx: &mut Context<Self>) {
        if self.shutting_down {
            return;
        }
        self.bridge.start(self.services.bridge.clone(), cx);
    }

    pub fn register_bridge(
        &mut self,
        client: &Client,
        cx: &mut Context<Self>,
    ) -> Result<BridgeRegistration, nocterm_ai::connection::BridgeRegistrationError> {
        if !self.ai_enabled(cx) {
            return Err(nocterm_ai::connection::BridgeRegistrationError::AiDisabled);
        }
        let registration = self.services.bridge.register()?;
        self.bridge.insert(registration.id, client.clone());
        self.start_bridge(cx);
        Ok(registration)
    }
    pub fn terminal_auth_request(
        &self,
        key: u64,
        method: &acp::AuthMethodTerminal,
    ) -> Option<TerminalAuthRequest> {
        let connection = self.connections.get(key)?;
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
    pub fn connect(
        &mut self,
        client: Client,
        chat_id: String,
        launch: AgentLaunch,
        cx: &mut Context<Self>,
    ) {
        if self.shutting_down || !self.ai_enabled(cx) {
            return;
        }
        let workdir = client
            .state(cx)
            .and_then(|state| state.workdir)
            .unwrap_or_else(|| {
                self.ai(cx)
                    .working_directory
                    .as_ref()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| self.services.workdir.clone())
            });
        if !workdir.is_absolute() {
            client.emit(
                SessionEvent::Failed(
                    "Working directory must be an existing absolute directory.".into(),
                ),
                cx,
            );
            return;
        }
        let isolated = self.ai(cx).sandbox == nocterm_ai::SandboxMode::Workspace;
        let key = self.connections.reserve_key(&launch, &workdir, isolated);
        let registration = match self.register_bridge(&client, cx) {
            Ok(registration) => registration,
            Err(error) => {
                client.emit(SessionEvent::Failed(error.to_string()), cx);
                return;
            }
        };
        let lease = SessionLease::new(client.id(), key, registration, self.releases.sender());
        client.emit(SessionEvent::Leased(lease), cx);
        let serial = self.connections.serial();
        let cancellation = nocterm_ai::ConnectionCancellation::default();
        let (delivery, startup_completion) = startup::start(
            startup::StartupConfig::for_connection(
                &self.services,
                launch.clone(),
                workdir.clone(),
                isolated,
                self.ai(cx).resources.clone(),
                cancellation.clone(),
            ),
            self.services.connector.clone(),
            cx.background_executor(),
        );
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
        let connecting = cx.spawn(async move |this, cx| {
            if let Ok(delivery) = delivery.await {
                let (result, effective, decision) = delivery.into_parts();
                let accepted = this
                    .update(cx, |this, cx| {
                        this.accept_startup(key, serial, result, effective, cx)
                    })
                    .unwrap_or(false);
                let _ = decision.send(accepted);
            }
        });
        self.connections.connecting(key, connecting);
    }
    fn accept_startup(
        &mut self,
        key: u64,
        serial: u64,
        result: Result<nocterm_ai::AgentConnection, nocterm_ai::AgentError>,
        effective: PathBuf,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self
            .connections
            .get(key)
            .is_some_and(|connection| connection.serial == serial)
            || !self.ai_enabled(cx)
            || self.shutting_down
        {
            return false;
        }
        match result {
            Ok(connection) => {
                let commands = connection.commands.clone();
                let info = connection.info.clone();
                let workdir = effective.clone();
                let events = cx.spawn(async move |this, cx| {
                    while let Ok(event) = connection.events.recv().await {
                        if this
                            .update(cx, |this, cx| this.event(key, serial, event, cx))
                            .is_err()
                        {
                            break;
                        }
                    }
                });
                for client in self.connections.connected(
                    key,
                    commands.clone(),
                    info.clone(),
                    effective,
                    events,
                ) {
                    client.emit(
                        SessionEvent::Connected {
                            connection: key,
                            commands: commands.clone(),
                            info: Box::new(info.clone()),
                            workdir: workdir.clone(),
                        },
                        cx,
                    );
                }
            }
            Err(error) => {
                self.stop_connection(key, &nocterm_ai::redact::redact(&error.to_string()), cx)
            }
        }
        true
    }
    fn event(&mut self, key: u64, serial: u64, event: AgentEvent, cx: &mut Context<Self>) {
        if let AgentEvent::Barrier(ack) = event {
            let _ = ack.send(());
            return;
        }
        let Some(connection) = self
            .connections
            .get(key)
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
                    self.usage.merge(agent, limit);
                    cx.notify();
                }
                let Some(connection) = self.connections.get(key) else {
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
        let Some(connection) = self.connections.get(key) else {
            return;
        };
        connection.cancellation.cancel();
        let clients: Vec<_> = connection.users.values().cloned().collect();
        for client in clients {
            client.emit(
                SessionEvent::Stopped {
                    connection: key,
                    message: message.to_owned(),
                },
                cx,
            );
        }
    }
}
