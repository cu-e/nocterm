//! Tells the agent access service about the servers the user connects to.
//!
//! A session of a saved connection is offered to the registry of live sessions first, and
//! then every online hub compares the server's `authorized_keys` with its trust registry.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{App, Entity};
use nocterm_connections::HostObserver;
use nocterm_hubs::Hubs;
use nocterm_ssh_keys::authorized_keys::{ConnectedHost, ConnectedHosts};

struct Reconciling {
    sessions: Arc<dyn ConnectedHosts>,
    hubs: Entity<Hubs>,
}

impl HostObserver for Reconciling {
    fn connected(&self, host: ConnectedHost, cx: &mut App) {
        let connection = host.connection.clone();
        self.sessions.connected(host);
        self.hubs
            .update(cx, |hubs, cx| hubs.reconcile_agent_host(&connection, cx));
    }
}

/// Connects the sessions of this computer's saved connections to the agent access service.
pub(crate) fn observe(
    sessions: Arc<dyn ConnectedHosts>,
    hubs: Entity<Hubs>,
    connections: &nocterm_connections::Installed,
) {
    connections.bind_host_observer(Rc::new(Reconciling { sessions, hubs }));
}
