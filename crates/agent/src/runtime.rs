use crate::thread::AgentThread;
use gpui_kit::{App, Context, Entity, EntityId, Global, Subscription, Task, WeakEntity};
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

pub struct AgentServices {
    pub connector: Arc<dyn AgentConnector>,
    pub bridge: Arc<dyn ToolBridge>,
    pub state_file: PathBuf,
    pub workdir: PathBuf,
}
pub(crate) struct RuntimeGlobal(pub Entity<Runtime>);
impl Global for RuntimeGlobal {}
struct Connection {
    launch: AgentLaunch,
    workdir: PathBuf,
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
            if !cx.ai_enabled() {
                this.registrations.clear();
                this.services.bridge.stop();
            }
        });
        let favorites =
            nocterm_ai::favorites::AgentStateFile::load(&services.state_file).unwrap_or_default();
        Self {
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
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        launch.hash(&mut hasher);
        workdir.hash(&mut hasher);
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
                for thread in connection.users.values().filter_map(WeakEntity::upgrade) {
                    thread.update(cx, |thread, cx| {
                        if thread.session.as_ref() == Some(&notification.session_id)
                            && thread.accept_updates
                            && !matches!(
                                notification.update,
                                acp::SessionUpdate::UserMessageChunk(_)
                            )
                        {
                            thread.state.apply(notification.update.clone());
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
        let keys: Vec<_> = self.connections.keys().copied().collect();
        for key in keys {
            self.stop_connection(key, "Application is closing.", cx);
        }
        self.registrations.clear();
        self.services.bridge.stop();
    }
}
