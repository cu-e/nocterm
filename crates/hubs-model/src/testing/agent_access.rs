//! In-memory agent access for tests of what lists and changes it.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::FutureExt as _;

use crate::address::HubId;
use crate::agent_access::{
    AgentAccessError, AgentAccessFuture, AgentAccessGrant, AgentCa, AgentGrants, AgentHostTrust,
    AgentTrustOverview, HostChoice, ServerTrust, TrustedServer,
};
use crate::error::HubError;
use crate::testing::FakeHub;

/// An in-memory [`AgentHostTrust`] and [`AgentGrants`]. Clones share one state, so a test
/// keeps a handle while a service owns another.
///
/// A server is reachable unless [`FakeAgentAccess::set_unreachable`] says otherwise. The
/// fake keeps the registry the way the real one does: a denied server that cannot be reached
/// stays pending until [`AgentHostTrust::reconcile`] reaches it.
#[derive(Clone, Default)]
pub struct FakeAgentAccess {
    state: Arc<Mutex<State>>,
    keys: Option<FakeHub>,
}

#[derive(Default)]
struct State {
    ca: Option<AgentCa>,
    servers: Vec<TrustedServer>,
    unreachable: BTreeSet<String>,
    line_lost: BTreeSet<String>,
    grants: Vec<AgentAccessGrant>,
    revoked: Vec<String>,
    failure: Option<AgentAccessError>,
    next_allow_error: Option<AgentAccessError>,
    reissued: usize,
    hosts: Vec<HostChoice>,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FakeAgentAccess {
    /// Records are encrypted under the account keys of `hub`: while they are closed
    /// ([`FakeHub::lock_keys`]) reading them fails with [`HubError::KeysLocked`].
    #[must_use]
    pub fn alongside(hub: &FakeHub) -> Self {
        Self {
            state: Arc::default(),
            keys: Some(hub.clone()),
        }
    }

    fn require_keys(&self, hub: &HubId) -> Result<(), AgentAccessError> {
        match &self.keys {
            Some(keys) if !keys.keys_open(hub) => Err(HubError::KeysLocked.into()),
            _ => Ok(()),
        }
    }

    /// The account has this CA from now on.
    pub fn set_ca(&self, ca: AgentCa) {
        lock(&self.state).ca = Some(ca);
    }

    /// The registry holds these servers from now on.
    pub fn set_servers(&self, servers: Vec<TrustedServer>) {
        lock(&self.state).servers = servers;
    }

    /// These chat accesses are active from now on.
    pub fn set_grants(&self, grants: Vec<AgentAccessGrant>) {
        lock(&self.state).grants = grants;
    }

    /// Whether the server of `connection` can be reached.
    pub fn set_unreachable(&self, connection: &str, unreachable: bool) {
        let mut state = lock(&self.state);
        if unreachable {
            state.unreachable.insert(connection.to_owned());
        } else {
            state.unreachable.remove(connection);
        }
    }

    /// These servers have an open session from now on.
    pub fn set_connected_hosts(&self, hosts: Vec<HostChoice>) {
        lock(&self.state).hosts = hosts;
    }

    /// Someone removed the CA line of `connection` on the server itself.
    pub fn lose_line(&self, connection: &str) {
        lock(&self.state).line_lost.insert(connection.to_owned());
    }

    /// Every call fails with `error` until [`FakeAgentAccess::recover`].
    pub fn fail_with(&self, error: AgentAccessError) {
        lock(&self.state).failure = Some(error);
    }

    /// Refuse one host edit while leaving the account registry and CA readable.
    pub fn fail_next_allow(&self, error: AgentAccessError) {
        lock(&self.state).next_allow_error = Some(error);
    }

    /// Calls work again.
    pub fn recover(&self) {
        lock(&self.state).failure = None;
    }

    /// The chat accesses revoked so far, in order.
    pub fn revoked(&self) -> Vec<String> {
        lock(&self.state).revoked.clone()
    }

    /// How many times the CA was replaced.
    pub fn reissued(&self) -> usize {
        lock(&self.state).reissued
    }
}

fn failed(state: &State) -> Result<(), AgentAccessError> {
    state.failure.clone().map_or(Ok(()), Err)
}

fn server<'a>(
    state: &'a mut State,
    connection: &str,
) -> Result<&'a mut TrustedServer, AgentAccessError> {
    state
        .servers
        .iter_mut()
        .find(|server| server.connection == connection)
        .ok_or_else(|| AgentAccessError::Unknown {
            connection: connection.to_owned(),
        })
}

impl AgentHostTrust for FakeAgentAccess {
    fn connected_hosts(&self) -> Vec<HostChoice> {
        lock(&self.state).hosts.clone()
    }

    fn overview(&self, hub: &HubId) -> AgentAccessFuture<AgentTrustOverview> {
        let (state, open) = (self.state.clone(), self.require_keys(hub));
        async move {
            open?;
            let state = lock(&state);
            failed(&state)?;
            Ok(AgentTrustOverview {
                ca: state.ca.clone(),
                servers: state.servers.clone(),
            })
        }
        .boxed()
    }

    fn allow(&self, _hub: &HubId, connection: &str) -> AgentAccessFuture<()> {
        let (state, connection) = (self.state.clone(), connection.to_owned());
        async move {
            let mut state = lock(&state);
            failed(&state)?;
            if let Some(error) = state.next_allow_error.take() {
                return Err(error);
            }
            if state.unreachable.contains(&connection) {
                return Err(AgentAccessError::NotConnected { connection });
            }
            state.line_lost.remove(&connection);
            if state.ca.is_none() {
                state.ca = Some(AgentCa {
                    fingerprint: "SHA256:fake".into(),
                    created_at_ms: 1,
                });
            }
            match server(&mut state, &connection) {
                Ok(known) => known.state = ServerTrust::Trusted,
                Err(_) => state.servers.push(TrustedServer {
                    user: "user".into(),
                    host: connection.clone(),
                    connection,
                    state: ServerTrust::Trusted,
                    needs_ca_update: false,
                    added_at_ms: 1,
                    added_by_device: "this device".into(),
                    last_verified_at_ms: Some(1),
                }),
            }
            Ok(())
        }
        .boxed()
    }

    fn deny(&self, _hub: &HubId, connection: &str) -> AgentAccessFuture<ServerTrust> {
        let (state, connection) = (self.state.clone(), connection.to_owned());
        async move {
            let mut state = lock(&state);
            failed(&state)?;
            let reachable = !state.unreachable.contains(&connection);
            let known = server(&mut state, &connection)?;
            known.state = if reachable {
                ServerTrust::Removed
            } else {
                ServerTrust::RemovalPending
            };
            Ok(known.state)
        }
        .boxed()
    }

    fn reconcile(&self, _hub: &HubId, connection: &str) -> AgentAccessFuture<ServerTrust> {
        let (state, connection) = (self.state.clone(), connection.to_owned());
        async move {
            let mut state = lock(&state);
            failed(&state)?;
            if state.unreachable.contains(&connection) {
                return Err(AgentAccessError::NotConnected { connection });
            }
            let lost = state.line_lost.contains(&connection);
            let known = server(&mut state, &connection)?;
            known.state = match known.state {
                ServerTrust::RemovalPending | ServerTrust::Removed => ServerTrust::Removed,
                _ if lost => ServerTrust::Drifted,
                _ => ServerTrust::Trusted,
            };
            // A connection gives a trusted server its own line.
            if known.state == ServerTrust::Trusted {
                known.needs_ca_update = false;
            }
            Ok(known.state)
        }
        .boxed()
    }

    fn reissue_ca(&self, _hub: &HubId) -> AgentAccessFuture<AgentCa> {
        let state = self.state.clone();
        async move {
            let mut state = lock(&state);
            failed(&state)?;
            state.reissued += 1;
            let ca = AgentCa {
                fingerprint: format!("SHA256:fake{}", state.reissued),
                created_at_ms: 2,
            };
            state.ca = Some(ca.clone());
            Ok(ca)
        }
        .boxed()
    }

    fn ca_line(&self, _hub: &HubId, connection: &str) -> AgentAccessFuture<String> {
        let (state, connection) = (self.state.clone(), connection.to_owned());
        async move {
            let state = lock(&state);
            failed(&state)?;
            state
                .ca
                .as_ref()
                .map(|ca| format!("cert-authority {} {connection}", ca.fingerprint))
                .ok_or_else(|| AgentAccessError::Unknown {
                    connection: String::new(),
                })
        }
        .boxed()
    }
}

impl AgentGrants for FakeAgentAccess {
    fn list(&self, hub: &HubId) -> AgentAccessFuture<Vec<AgentAccessGrant>> {
        let (state, open) = (self.state.clone(), self.require_keys(hub));
        async move {
            open?;
            let state = lock(&state);
            failed(&state)?;
            Ok(state.grants.clone())
        }
        .boxed()
    }

    fn revoke(&self, _hub: &HubId, grant_id: &str) -> AgentAccessFuture<()> {
        let (state, grant_id) = (self.state.clone(), grant_id.to_owned());
        async move {
            let mut state = lock(&state);
            failed(&state)?;
            state.grants.retain(|grant| grant.grant_id != grant_id);
            state.revoked.push(grant_id);
            Ok(())
        }
        .boxed()
    }
}
