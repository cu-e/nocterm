//! Agent lifecycle and durable chat orchestration.
//!
//! Owns: pure admission policy, connection/lease owners, document identity, bounded
//! keyed write admission, lazy history hydration and usage projections.
//! Does not own: settings storage, history/favorites files, ACP subprocesses or views.
//! Ports used: AgentConnector, ToolBridge, AgentStateRepository, ChatRepository,
//! AccountUsageSource, WorkdirResolver, explicit SettingsHandle and host-supplied ShutdownCoordinator
//! at installation.
//! Ports provided: SessionClient/SessionEvent, HistoryReader and synchronous quit capture.
//! Settings section: ai consumer. Globals: none.
//! Invariants: one current document owner per ID; delete seals an ID; persistence
//! admission and physical publication are separate revision-bearing events. Registry
//! stamps change only after writer acceptance; final work reads no App and joins
//! captured admission failures with publication failures. Startup, notices, permissions and
//! process cleanup keep their original paid native owners: each certificate reaches one chat
//! exactly once and a refused or dropped certificate refunds its policy. Shared cleanup
//! diagnostics retain the original paid native failure through standard source traversal.
//! A connection's event receiver and process cleanup are released before its close is
//! reported, so the chat may immediately reopen the same original source.

mod handle;
pub use handle::RuntimeHandle;
mod admission;
mod bridge;
mod catalog;
mod client;
mod releases;
mod usage;
pub use catalog::PreparedHistory;
mod connections;
mod documents;
mod event_pump;
pub use documents::{CaptureAdmission, DocumentWriter};
mod lease;
mod lifecycle;
mod process_cleanup;
mod startup;
mod state;
use futures::future::BoxFuture;
mod storage;
pub use client::{CaptureError, Client, ClientState, SessionClient, SessionEvent};
use gpui::{App, Context, Entity, EntityId, Task};
pub use lease::SessionLease;
use nocterm_ai::{
    AgentConnector, AgentLaunch, BridgeRegistration, ToolBridge,
    connection::{
        ManagedAgentConnection, ManagedAgentEvent, ManagedCommandsHandle, ManagedWorkspaceHandle,
    },
    history::source::SourceLifetime,
    model::{auth::TerminalSetup, controls::ConfigOption, notice::SessionUpdate},
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
    pub retention: nocterm_ai::model::retention::RetentionAuthority,
    pub settings: nocterm_store::SettingsHandle,
    pub connector: Arc<dyn AgentConnector>,
    pub bridge: Arc<dyn ToolBridge>,
    pub storage: AgentStorage,
    /// Account usage is read through a host-injected native capability.
    pub usage_source: Arc<dyn nocterm_ai::usage::AccountUsageSource>,
    pub workdir_resolver: Arc<dyn nocterm_ai::workspace::WorkdirResolver>,
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
struct Connection {
    chat_id: String,
    launch: AgentLaunch,
    workdir: PathBuf,
    /// Whether the process runs isolated; a changed setting restarts it.
    isolated: bool,
    commands: Option<ManagedCommandsHandle>,
    workspace: Option<ManagedWorkspaceHandle>,
    source: SourceLifetime,
    users: HashMap<EntityId, Client>,
    serial: u64,
    _events: Option<event_pump::EventPump>,
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

    pub fn new(mut services: RuntimeServices, cx: &mut Context<Self>) -> Self {
        services.settings.register::<nocterm_ai::AiSettings>(cx);
        for warning in
            nocterm_ai::AgentRegistry::new(services.settings.setting::<nocterm_ai::AiSettings>(cx))
                .warnings
        {
            tracing::warn!(message=%nocterm_ai::redact::redact(&warning),"Ignoring AI agent configuration");
        }
        let state = state::AgentStateStore::new(&services.storage, cx);
        let chats = documents::ChatStore::new(&mut services.storage, cx);
        let releases = releases::LeaseInbox::new(cx);
        let usage = usage::UsageLimits::new(services.usage_source.clone());
        Self {
            state,
            chats,
            usage,
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
        if !self.ai_enabled(cx) {
            for id in self.chats.capture_and_detach_all(cx) {
                self.admission.withdraw(id);
            }
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
        options: &[ConfigOption],
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
        option: &ConfigOption,
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
        let registry = nocterm_ai::AgentRegistry::new(self.ai(cx));
        let Some(launch) = registry.get(agent) else {
            return;
        };
        self.usage
            .refresh(nocterm_ai::usage::UsageAgent::from(launch), cx);
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
        method: &TerminalSetup,
        label: &str,
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
            title: format!("Sign in: {label}"),
        })
    }
    fn startup_workdir(&self, client: &Client, cx: &App) -> PathBuf {
        client
            .state(cx)
            .and_then(|state| state.workdir)
            .unwrap_or_else(|| {
                self.ai(cx)
                    .working_directory
                    .as_ref()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| self.services.workdir.clone())
            })
    }
    /// Copied launch facts are paid against the chat's document before any registration exists.
    fn prepare_startup(
        &self,
        client: &Client,
        launch: &AgentLaunch,
        workdir: &std::path::Path,
        isolated: bool,
        cx: &mut Context<Self>,
    ) -> Option<nocterm_ai::connection::ManagedStartup> {
        let resources = self.ai(cx).resources.clone();
        let isolation = isolated.then(|| nocterm_ai::workspace::IsolationPaths {
            private_dirs: self.services.private_dirs.clone(),
            shared_dirs: self.services.shared_dirs.clone(),
        });
        let request = nocterm_ai::connection::ManagedStartupRequest {
            launch,
            working_directory: workdir,
            create_private: workdir == self.services.workdir,
            isolation: isolation.as_ref(),
            terminal_auth: self.services.terminal_auth,
            resources: &resources,
        };
        match client.prepare_startup(request, cx) {
            Ok(startup) => Some(startup),
            Err(error) => {
                client.emit(
                    SessionEvent::Failed(nocterm_ai::redact::redact(&error.to_string())),
                    cx,
                );
                None
            }
        }
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
        let workdir = self.startup_workdir(&client, cx);
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
        let Some(startup) = self.prepare_startup(&client, &launch, &workdir, isolated, cx) else {
            return;
        };
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
        let cancellation = startup.cancellation().clone();
        let source = startup.lifetime();
        let (delivery, startup_completion) = startup::start(
            startup::ManagedStartupConfig::new(
                startup,
                self.services.workdir_resolver.clone(),
                self.services.connector.clone(),
            ),
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
                workspace: None,
                source,
                users: HashMap::from([(client.id(), client.clone())]),
                serial,
                _events: None,
                _connecting: None,
                cancellation,
                startup_completion: Some(startup_completion),
            },
        );
        let connecting = cx.spawn(async move |this, cx| {
            if let Ok(delivery) = delivery.await {
                let (result, workspace, decision) = delivery.into_parts();
                let accepted = this
                    .update(cx, |this, cx| {
                        this.accept_startup(key, serial, result, workspace, cx)
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
        result: Result<ManagedAgentConnection, startup::StartupError>,
        workspace: Option<ManagedWorkspaceHandle>,
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
                let (info, commands, mut receiver) = connection.into_parts();
                let workdir = workspace
                    .as_ref()
                    .map(|workspace| workspace.workspace().root.clone())
                    .or_else(|| {
                        self.connections
                            .get(key)
                            .map(|connection| connection.workdir.clone())
                    })
                    .unwrap_or_default();
                let (mut signals, control) = event_pump::signals();
                let task = cx.spawn(async move |this, cx| {
                    while let Some(event) = signals
                        .until_stopped(futures::future::poll_fn(|task| receiver.poll_event(task)))
                        .await
                    {
                        if this
                            .update(cx, |this, cx| this.event(key, serial, event, cx))
                            .is_err()
                        {
                            break;
                        }
                    }
                    drop(receiver);
                    signals.finished();
                });
                let events = event_pump::EventPump::new(task, control);
                let clients = match self.connections.connected(
                    key,
                    commands.clone(),
                    workspace,
                    workdir.clone(),
                    events,
                    cx.background_executor(),
                ) {
                    Ok(clients) => clients,
                    Err(error) => {
                        commands.shutdown();
                        self.stop_connection(
                            key,
                            &nocterm_ai::redact::redact(&error.to_string()),
                            cx,
                        );
                        return true;
                    }
                };
                // The information certificate is consumed once, by the connection's only user.
                if let Some(client) = clients.into_iter().next() {
                    client.emit(
                        SessionEvent::Connected {
                            connection: key,
                            commands,
                            info,
                            workdir,
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
    fn event(&mut self, key: u64, serial: u64, event: ManagedAgentEvent, cx: &mut Context<Self>) {
        if let ManagedAgentEvent::Barrier(barrier) = event {
            barrier.acknowledge();
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
            ManagedAgentEvent::Barrier(_) => unreachable!("handled before connection routing"),
            ManagedAgentEvent::Session(notice) => {
                if let SessionUpdate::Usage(update) = &notice.notice().update
                    && !update.rate_limits.is_empty()
                {
                    let agent = connection.launch.id.clone();
                    for limit in &update.rate_limits {
                        self.usage.merge(agent.clone(), limit.clone());
                    }
                    cx.notify();
                }
                let Some(connection) = self.connections.get(key) else {
                    return;
                };
                // A notice is delivered once. With a single user, pre-open controls reach it
                // before it owns a session, so they can be kept until the open completes.
                let session = &notice.notice().session;
                let client = connection
                    .users
                    .values()
                    .find(|client| {
                        client
                            .state(cx)
                            .is_some_and(|state| state.session.as_ref() == Some(session))
                    })
                    .or_else(|| {
                        (connection.users.len() == 1)
                            .then(|| connection.users.values().next())
                            .flatten()
                    })
                    .cloned();
                if let Some(client) = client {
                    client.emit(SessionEvent::Update(notice), cx);
                }
            }
            ManagedAgentEvent::Permission { request, respond } => {
                let client = connection
                    .users
                    .values()
                    .find(|client| {
                        client.state(cx).is_some_and(|state| {
                            state.session.as_ref() == Some(&request.permission().session)
                        })
                    })
                    .cloned();
                // A request no thread takes is answered `Cancelled` on drop.
                if let Some(client) = client {
                    client.emit(SessionEvent::Permission { request, respond }, cx);
                }
            }
            ManagedAgentEvent::Failed(error) => {
                self.stop_connection(key, &nocterm_ai::redact::redact(&error.to_string()), cx)
            }
            ManagedAgentEvent::Exited { code, stderr_tail } => self.stop_connection(
                key,
                &format!(
                    "Agent exited ({code:?}). {}",
                    nocterm_ai::redact::redact(stderr_tail.as_str())
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
