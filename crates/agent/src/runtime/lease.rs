//! A thread's hold on an agent connection, released when it is dropped.
use gpui_kit::EntityId;
use nocterm_ai::{AgentCommands, BridgeRegistration, acp};
use std::{path::PathBuf, sync::Arc};

/// The connection, session and tool registration one thread is using.
///
/// Dropping the lease releases them: every path that ends a thread's use of
/// a connection, error branches included, closes it.
pub(crate) struct SessionLease {
    pub session: Option<acp::SessionId>,
    pub commands: Option<Arc<dyn AgentCommands>>,
    pub registration: Option<BridgeRegistration>,
    pub connection_key: u64,
    pub workdir: Option<PathBuf>,
    owner: EntityId,
    releases: async_channel::Sender<ReleasedLease>,
}

/// What the runtime needs to close the connection of a dropped lease.
pub(crate) struct ReleasedLease {
    pub owner: EntityId,
    pub session: Option<acp::SessionId>,
    pub commands: Option<Arc<dyn AgentCommands>>,
    pub registration: Option<BridgeRegistration>,
    pub connection_key: u64,
}

impl SessionLease {
    pub(crate) fn new(
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
