//! A thread's hold on an agent connection, released when it is dropped.
use gpui::EntityId;
use nocterm_ai::{AgentCommands, BridgeRegistration, model::ids::SessionId};
use std::{path::PathBuf, sync::Arc};

/// The connection, session and tool registration one thread is using.
///
/// Dropping the lease releases them: every path that ends a thread's use of
/// a connection, error branches included, closes it.
pub struct SessionLease {
    session: Option<SessionId>,
    commands: Option<Arc<dyn AgentCommands>>,
    registration: Option<BridgeRegistration>,
    connection_key: u64,
    workdir: Option<PathBuf>,
    owner: EntityId,
    releases: async_channel::Sender<ReleasedLease>,
}

/// What the runtime needs to close the connection of a dropped lease.
pub struct ReleasedLease {
    pub owner: EntityId,
    pub session: Option<SessionId>,
    pub commands: Option<Arc<dyn AgentCommands>>,
    pub registration: Option<BridgeRegistration>,
    pub connection_key: u64,
}

impl SessionLease {
    pub fn new(
        owner: EntityId,
        connection_key: u64,
        registration: BridgeRegistration,
        releases: async_channel::Sender<ReleasedLease>,
    ) -> Self {
        Self {
            session: None,
            commands: None,
            registration: Some(registration),
            connection_key,
            workdir: None,
            owner,
            releases,
        }
    }
    pub fn session(&self) -> &Option<SessionId> {
        &self.session
    }
    pub fn commands(&self) -> &Option<Arc<dyn AgentCommands>> {
        &self.commands
    }
    pub fn registration(&self) -> &Option<BridgeRegistration> {
        &self.registration
    }
    pub fn workdir(&self) -> &Option<PathBuf> {
        &self.workdir
    }
    pub fn connection_key(&self) -> u64 {
        self.connection_key
    }
    /// Bind the connection's commands and the workdir the session opens in.
    /// The session driver orders this before [`Self::accept_session`].
    pub fn begin_open(&mut self, commands: Arc<dyn AgentCommands>, workdir: PathBuf) {
        self.commands = Some(commands);
        self.workdir = Some(workdir);
    }
    /// Record the opened session; a fresh session reports its own workdir.
    pub fn accept_session(&mut self, session: SessionId, fresh_workdir: Option<PathBuf>) {
        if let Some(workdir) = fresh_workdir {
            self.workdir = Some(workdir);
        }
        self.session = Some(session);
    }
}

impl Drop for SessionLease {
    fn drop(&mut self) {
        // `Drop` has no `App` to update the runtime with, so the release is
        // queued and the runtime's own task applies it.
        let _ = self.releases.try_send(ReleasedLease {
            owner: self.owner,
            session: self.session.take(),
            commands: self.commands.take(),
            registration: self.registration.take(),
            connection_key: self.connection_key,
        });
    }
}
