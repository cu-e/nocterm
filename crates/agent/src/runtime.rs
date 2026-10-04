use crate::thread::AgentThread;
use gpui_kit::{App, Context, Entity, EntityId, Global, Subscription, Task, WeakEntity, Window};
use nocterm_ai::{
    AgentCommands, AgentConnector, AgentEvent, AgentInfo, AgentLaunch, BridgeRegistration,
    ConnectRequest, ToolBridge, acp,
};
use nocterm_ui::{ActiveAi as _, ActiveSettings as _, SettingsStore};
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
pub type TerminalAuthOpener = Arc<
    dyn Fn(
            WeakEntity<nocterm_workspace::Workspace>,
            TerminalAuthRequest,
            &mut Window,
            &mut App,
        ) -> futures::channel::oneshot::Receiver<Result<(), String>>
        + Send
        + Sync,
>;

pub struct AgentServices {
    pub connector: Arc<dyn AgentConnector>,
    pub bridge: Arc<dyn ToolBridge>,
    pub state_file: PathBuf,
    /// Saved chats, one file each.
    pub chats_dir: PathBuf,
    /// Where Codex keeps its session logs, read for its plan limits.
    pub codex_home: Option<PathBuf>,
    pub workdir: PathBuf,
    pub terminal_auth: Option<TerminalAuthOpener>,
    /// nocterm's own directories (settings, vault, chats), hidden from
    /// isolated agents.
    pub private_dirs: Vec<PathBuf>,
    /// Directories isolated agents must still reach, such as the terminal
    /// tools' socket directory.
    pub shared_dirs: Vec<PathBuf>,
}
pub(crate) struct RuntimeGlobal(pub Entity<Runtime>);
impl Global for RuntimeGlobal {}
struct Connection {
    launch: AgentLaunch,
    workdir: PathBuf,
    /// Whether the process runs isolated; a changed setting restarts it.
    isolated: bool,
    commands: Option<Arc<dyn AgentCommands>>,
    info: Option<AgentInfo>,
    users: HashMap<EntityId, WeakEntity<AgentThread>>,
    serial: u64,
    _events: Option<Task<()>>,
    _connecting: Option<Task<()>>,
}
pub(crate) struct Runtime {
    pub services: AgentServices,
    pub favorites: nocterm_ai::favorites::AgentStateFile,
    pub favorites_error: Option<String>,
    favorites_revision: u64,
    favorites_writer: Option<Task<()>>,
    connections: HashMap<u64, Connection>,
    pub registrations: HashMap<u64, WeakEntity<AgentThread>>,
    /// Chats read from disk and not yet shown by a panel; `None` while reading.
    pub(crate) saved_chats: Option<Vec<nocterm_ai::history::SavedChat>>,
    /// Latest unsaved snapshot of each chat, written in order by `chat_writer`.
    chat_writes: HashMap<String, Option<nocterm_ai::history::SavedChat>>,
    chat_writer: Option<Task<()>>,
    /// Deleted chats, never written again by a late save.
    deleted_chats: std::collections::HashSet<String>,
    _loading_chats: Option<Task<()>>,
    /// Plan limits last reported, by agent.
    limits: HashMap<String, nocterm_ai::usage::Limits>,
    /// Agents whose limits are being read.
    reading_limits: std::collections::HashSet<String>,
    serial: u64,
    _settings: Subscription,
    _bridge: Option<Task<()>>,
}
impl Runtime {
    pub(crate) fn global(cx: &App) -> Entity<Self> {
        cx.global::<RuntimeGlobal>().0.clone()
    }
    pub(crate) fn new(services: AgentServices, cx: &mut Context<Self>) -> Self {
        for warning in nocterm_ai::AgentRegistry::new(&cx.settings().ai).warnings {
            tracing::warn!(message=%nocterm_ai::redact::redact(&warning),"Ignoring AI agent configuration");
        }
        let settings = cx.observe_global::<SettingsStore>(|this, cx| {
            let registry = nocterm_ai::AgentRegistry::new(&cx.settings().ai);
            for warning in &registry.warnings{tracing::warn!(message=%nocterm_ai::redact::redact(warning),"Ignoring AI agent configuration");}
            let keys: Vec<_> = this
                .connections
                .iter()
                .filter(|(_, connection)| registry.get(&connection.launch.id).is_none())
                .map(|(key, _)| *key)
                .collect();
            for key in keys {
                this.stop_connection(key, "Agent configuration changed. Start a new chat.", cx);
            }
            // Isolation is a security boundary: it applies to running agents
            // at once rather than to the next chat.
            let isolated = cx.settings().ai.sandbox == nocterm_settings::SandboxMode::Workspace;
            let keys: Vec<_> = this
                .connections
                .iter()
                .filter(|(_, connection)| connection.isolated != isolated)
                .map(|(key, _)| *key)
                .collect();
            for key in keys {
                this.stop_connection(
                    key,
                    if isolated {
                        "Agent isolation was turned on. Restart the chat to continue isolated."
                    } else {
                        "Agent isolation was turned off. Restart the chat to continue."
                    },
                    cx,
                );
            }
            if !cx.ai_enabled() {
                this.registrations.clear();
                this.services.bridge.stop();
            }
        });
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
        Self {
            saved_chats: None,
            chat_writes: HashMap::new(),
            chat_writer: None,
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
            registrations: HashMap::new(),
            serial: 0,
            _settings: settings,
            _bridge: None,
        }
    }
    pub(crate) fn toggle_favorite(
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
    pub(crate) fn set_last_agent(&mut self, agent: &str, cx: &mut Context<Self>) {
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
    pub(crate) fn limits(&self, agent: &str) -> Option<&nocterm_ai::usage::Limits> {
        self.limits.get(agent).filter(|limits| !limits.is_empty())
    }
    /// Reads `agent`'s plan limits again, from agents that keep them in their
    /// own files. Others report them along with usage.
    pub(crate) fn refresh_limits(&mut self, agent: &str, cx: &mut Context<Self>) {
        let codex = nocterm_ai::AgentRegistry::new(&cx.settings().ai)
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
    /// Saved chats for the first panel that asks once they are read; later
    /// panels get none, so two windows never write the same chat.
    pub(crate) fn take_saved_chats(&mut self) -> Option<Vec<nocterm_ai::history::SavedChat>> {
        self.saved_chats.as_mut().map(std::mem::take)
    }
    /// Queues `chat` to be written, replacing an older queued snapshot.
    pub(crate) fn save_chat(
        &mut self,
        chat: nocterm_ai::history::SavedChat,
        cx: &mut Context<Self>,
    ) {
        if self.deleted_chats.contains(&chat.id) {
            return;
        }
        self.chat_writes.insert(chat.id.clone(), Some(chat));
        self.write_chats(cx);
    }
    /// Queues the removal of chat `id`.
    pub(crate) fn delete_chat(&mut self, id: String, cx: &mut Context<Self>) {
        self.deleted_chats.insert(id.clone());
        self.chat_writes.insert(id, None);
        self.write_chats(cx);
    }
    /// Writes every open chat and queued change now, before the application
    /// exits.
    fn flush_chats(&mut self, cx: &mut Context<Self>) {
        let mut writes = std::mem::take(&mut self.chat_writes);
        for thread in self
            .connections
            .values()
            .flat_map(|connection| connection.users.values())
            .filter_map(WeakEntity::upgrade)
        {
            if let Some(chat) = thread.read(cx).snapshot()
                && !self.deleted_chats.contains(&chat.id)
            {
                writes.insert(chat.id.clone(), Some(chat));
            }
        }
        for (id, chat) in writes {
            let result = match chat {
                Some(chat) => nocterm_ai::history::save(&self.services.chats_dir, &chat),
                None => nocterm_ai::history::delete(&self.services.chats_dir, &id),
            };
            if let Err(error) = result {
                tracing::warn!(%error, "could not save agent chat");
            }
        }
    }
    fn write_chats(&mut self, cx: &mut Context<Self>) {
        if self.chat_writer.is_some() {
            return;
        }
        let dir = self.services.chats_dir.clone();
        self.chat_writer = Some(cx.spawn(async move |this, cx| {
            loop {
                let next = this.update(cx, |this, _| {
                    let id = this.chat_writes.keys().next().cloned();
                    match id {
                        Some(id) => this.chat_writes.remove_entry(&id),
                        None => {
                            this.chat_writer = None;
                            None
                        }
                    }
                });
                let Ok(Some((id, chat))) = next else {
                    return;
                };
                let dir = dir.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        match chat {
                            Some(chat) => nocterm_ai::history::save(&dir, &chat),
                            None => nocterm_ai::history::delete(&dir, &id),
                        }
                    })
                    .await;
                if let Err(error) = result {
                    tracing::warn!(%error, "could not save agent chat");
                }
            }
        }));
    }
    pub(crate) fn start_bridge(&mut self, cx: &mut Context<Self>) {
        if self._bridge.is_some() {
            return;
        }
        let calls = self.services.bridge.calls();
        self._bridge = Some(cx.spawn(async move |this, cx| {
            while let Ok(call) = calls.recv().await {
                let thread = this
                    .read_with(cx, |this, _| {
                        this.registrations
                            .get(&call.registration_id)
                            .and_then(WeakEntity::upgrade)
                    })
                    .ok()
                    .flatten();
                if let Some(thread) = thread {
                    thread.update(cx, |thread, cx| thread.handle_tool(call, cx));
                } else {
                    let _ = call
                        .respond
                        .send(Err("Chat was closed or AI is disabled.".into()));
                }
            }
        }));
    }
    pub(crate) fn register_bridge(
        &mut self,
        thread: &Entity<AgentThread>,
        cx: &mut Context<Self>,
    ) -> Result<BridgeRegistration, String> {
        if !cx.ai_enabled() {
            return Err("AI is disabled.".into());
        }
        let registration = self.services.bridge.register()?;
        self.registrations
            .insert(registration.id, thread.downgrade());
        self.start_bridge(cx);
        Ok(registration)
    }
    pub(crate) fn terminal_auth_request(
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
    pub(crate) fn connect(
        &mut self,
        thread: Entity<AgentThread>,
        launch: AgentLaunch,
        fresh: bool,
        cx: &mut Context<Self>,
    ) {
        if !cx.ai_enabled() {
            return;
        }
        let workdir = cx
            .settings()
            .ai
            .working_directory
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.services.workdir.clone());
        let isolated = cx.settings().ai.sandbox == nocterm_settings::SandboxMode::Workspace;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        launch.hash(&mut hasher);
        workdir.hash(&mut hasher);
        isolated.hash(&mut hasher);
        if fresh {
            self.serial += 1;
            self.serial.hash(&mut hasher);
        }
        let key = hasher.finish();
        thread.update(cx, |thread, _| thread.connection_key = Some(key));
        if let Some(connection) = self.connections.get_mut(&key) {
            connection
                .users
                .insert(thread.entity_id(), thread.downgrade());
            if let (Some(commands), Some(info)) = (&connection.commands, &connection.info) {
                thread.update(cx, |thread, cx| {
                    thread.create_session(commands.clone(), info.clone(), workdir, cx)
                });
            }
            return;
        }
        self.serial += 1;
        let serial = self.serial;
        self.connections.insert(
            key,
            Connection {
                launch: launch.clone(),
                workdir: workdir.clone(),
                isolated,
                commands: None,
                info: None,
                users: HashMap::from([(thread.entity_id(), thread.downgrade())]),
                serial,
                _events: None,
                _connecting: None,
            },
        );
        let connector = self.services.connector.clone();
        let private_workdir = workdir == self.services.workdir;
        let terminal_auth = self.services.terminal_auth.is_some();
        let sandbox = isolated.then(|| {
            nocterm_ai::sandbox::SandboxPolicy::new(
                &workdir,
                std::env::home_dir().as_deref(),
                &self.services.private_dirs,
                &self.services.shared_dirs,
            )
        });
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
                })
                .await
        });
        let connecting = cx.spawn(async move |this, cx| {
            let result = future.await;
            let _ = this.update(cx, |this, cx| {
                if !this
                    .connections
                    .get(&key)
                    .is_some_and(|connection| connection.serial == serial)
                    || !cx.ai_enabled()
                {
                    if let Ok(connection) = result {
                        connection.commands.shutdown();
                    }
                    return;
                }
                match result {
                    Ok(connection) => {
                        let slot = this.connections.get_mut(&key).expect("checked connection");
                        slot.commands = Some(connection.commands.clone());
                        slot.info = Some(connection.info.clone());
                        for thread in slot.users.values().filter_map(WeakEntity::upgrade) {
                            thread.update(cx, |thread, cx| {
                                thread.create_session(
                                    connection.commands.clone(),
                                    connection.info.clone(),
                                    slot.workdir.clone(),
                                    cx,
                                )
                            });
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
            });
        });
        if let Some(connection) = self.connections.get_mut(&key) {
            connection._connecting = Some(connecting);
        }
    }
    fn event(&mut self, key: u64, serial: u64, event: AgentEvent, cx: &mut Context<Self>) {
        let Some(connection) = self
            .connections
            .get(&key)
            .filter(|connection| connection.serial == serial)
        else {
            return;
        };
        match event {
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
                for thread in connection.users.values().filter_map(WeakEntity::upgrade) {
                    thread.update(cx, |thread, cx| {
                        if thread.session.as_ref() == Some(&notification.session_id)
                            && thread.accept_updates
                            && !matches!(
                                notification.update,
                                acp::SessionUpdate::UserMessageChunk(_)
                            )
                        {
                            let change = thread.state.apply(notification.update.clone());
                            if change == nocterm_ai::thread::ThreadChange::Metadata {
                                thread.persist(cx);
                            }
                            cx.notify();
                        }
                    });
                }
            }
            AgentEvent::Permission { request, respond } => {
                let thread = connection
                    .users
                    .values()
                    .filter_map(WeakEntity::upgrade)
                    .find(|thread| thread.read(cx).session.as_ref() == Some(&request.session_id));
                if let Some(thread) = thread {
                    thread.update(cx, |thread, cx| thread.permission(request, respond, cx));
                } else {
                    let _ = respond.send(acp::RequestPermissionOutcome::Cancelled);
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
        if let Some(connection) = self.connections.remove(&key) {
            if let Some(commands) = connection.commands {
                commands.shutdown();
            }
            for thread in connection.users.values().filter_map(WeakEntity::upgrade) {
                thread.update(cx, |thread, cx| {
                    thread.fail(message, cx);
                    if !cx.ai_enabled() {
                        thread.state = Default::default();
                        thread.attachments.clear();
                        thread.images.clear();
                    }
                });
            }
        }
    }
    pub(crate) fn release(&mut self, id: EntityId, key: Option<u64>, registration: Option<u64>) {
        if let Some(registration) = registration {
            self.registrations.remove(&registration);
        }
        if let Some(key) = key
            && let Some(connection) = self.connections.get_mut(&key)
        {
            connection.users.remove(&id);
            if connection.users.is_empty()
                && let Some(connection) = self.connections.remove(&key)
                && let Some(commands) = connection.commands
            {
                commands.shutdown();
            }
        }
    }
    pub(crate) fn shutdown(&mut self, cx: &mut Context<Self>) {
        self.flush_chats(cx);
        let keys: Vec<_> = self.connections.keys().copied().collect();
        for key in keys {
            self.stop_connection(key, "Application is closing.", cx);
        }
        self.registrations.clear();
        self.services.bridge.stop();
    }
}
