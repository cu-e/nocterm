//! An in-memory hub for tests: one fake behind all three access ports.
//!
//! Enable `test-support` in a consumer's dev-dependencies. The fake does no I/O and keeps its
//! "hubs" in memory. A call completes when it is polled; [`FakeHub::hold`] makes the next call
//! of one kind wait until the test releases it, so a test can observe the state in between.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::FutureExt as _;
use futures::channel::oneshot;
use nocterm_core::error::SharedCause;
use nocterm_session::testing::ScriptedTransport;
use nocterm_session::{Secret, Transport};

use crate::address::{HubAddress, HubId};
use crate::error::HubError;
use crate::ports::{HubFuture, HubSessions, HubSignIn, HubTrust, OidcSignIn};
use crate::records::{
    Account, HubEndpointSpec, HubInfo, HubSessionId, HubSessionState, HubSessionSummary,
    HubTerminalTarget, NodeSummary, OrgId,
};
use crate::trust::{ProbedHub, TrustPolicy};

mod server;

pub use server::{FakeServer, FakeUser};

/// The protocol version the fake client speaks.
pub const CLIENT_PROTOCOL: u32 = 1;

/// The kinds of call a test can observe, hold or make fail.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FakeOp {
    Probe,
    Connect,
    Password,
    Oidc,
    SignOut,
    Account,
    Nodes,
    Sessions,
    End,
}

/// One call the fake received. Secrets are not recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FakeCall {
    Probe(HubAddress),
    Connect(HubId),
    Password { hub: HubId, username: String },
    Oidc { hub: HubId, provider: String },
    SignOut(HubId),
    Forget(HubId),
    Account(HubId),
    Nodes(HubId, OrgId),
    Sessions(HubId, OrgId),
    End(HubId, HubSessionId),
    Terminal(HubId, HubTerminalTarget),
}

/// Why a fake hub did not answer.
#[derive(Debug, thiserror::Error)]
#[error("the fake hub is offline")]
pub struct FakeOffline;

/// An in-memory hub implementing [`HubTrust`], [`HubSignIn`] and [`HubSessions`].
///
/// Clones share one state, so a test keeps a handle while a service owns another.
#[derive(Clone, Default)]
pub struct FakeHub {
    state: Arc<Mutex<State>>,
    terminals: ScriptedTransport,
}

#[derive(Default)]
struct State {
    servers: BTreeMap<HubAddress, FakeServer>,
    /// Hubs this device holds credentials for, and who it is signed in as.
    credentials: BTreeMap<HubId, String>,
    revoked: BTreeSet<HubId>,
    calls: Vec<FakeCall>,
    failures: VecDeque<(FakeOp, HubError)>,
    holds: Vec<(FakeOp, oneshot::Receiver<()>)>,
    oidc: Option<oneshot::Sender<Result<String, HubError>>>,
    oidc_cancelled: usize,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Lets the held call continue when [`Release::release`] is called or the value is dropped.
#[derive(Debug)]
pub struct Release(oneshot::Sender<()>);

impl Release {
    pub fn release(self) {
        let _ = self.0.send(());
    }
}

impl FakeHub {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a server reachable at `server.address`.
    #[must_use]
    pub fn with_server(self, server: FakeServer) -> Self {
        lock(&self.state)
            .servers
            .insert(server.address.clone(), server);
        self
    }

    /// Changes a server through `change`; panics when the address is not served.
    pub fn edit_server(&self, address: &HubAddress, change: impl FnOnce(&mut FakeServer)) {
        let mut state = lock(&self.state);
        change(state.servers.get_mut(address).expect("a served address"));
    }

    /// Pretends this device already signed in to `hub` as `username`.
    pub fn sign_in_as(&self, hub: &HubId, username: &str) {
        lock(&self.state)
            .credentials
            .insert(hub.clone(), username.to_owned());
    }

    /// Makes the hub revoke this device: its next call finds the device revoked.
    pub fn revoke_device(&self, hub: &HubId) {
        lock(&self.state).revoked.insert(hub.clone());
    }

    pub fn is_signed_in(&self, hub: &HubId) -> bool {
        lock(&self.state).credentials.contains_key(hub)
    }

    /// The transport behind every terminal this hub hands out. Queue sessions on it, in the
    /// order the terminals will be opened, to script their events.
    pub fn terminals(&self) -> ScriptedTransport {
        self.terminals.clone()
    }

    /// Every call received, oldest first.
    pub fn calls(&self) -> Vec<FakeCall> {
        lock(&self.state).calls.clone()
    }

    /// How many calls of one kind arrived.
    pub fn count(&self, op: FakeOp) -> usize {
        lock(&self.state)
            .calls
            .iter()
            .filter(|call| call.op() == Some(op))
            .count()
    }

    /// The next call of kind `op` fails with `error`.
    pub fn fail_next(&self, op: FakeOp, error: HubError) {
        lock(&self.state).failures.push_back((op, error));
    }

    /// The next call of kind `op` waits until the returned handle releases it.
    #[must_use]
    pub fn hold(&self, op: FakeOp) -> Release {
        let (release, wait) = oneshot::channel();
        lock(&self.state).holds.push((op, wait));
        Release(release)
    }

    /// Finishes the pending browser sign-in as `username`.
    pub fn complete_oidc(&self, username: &str) {
        if let Some(done) = lock(&self.state).oidc.take() {
            let _ = done.send(Ok(username.to_owned()));
        }
    }

    /// Fails the pending browser sign-in.
    pub fn fail_oidc(&self, error: HubError) {
        if let Some(done) = lock(&self.state).oidc.take() {
            let _ = done.send(Err(error));
        }
    }

    /// How many browser sign-ins were dropped before they finished.
    pub fn cancelled_oidc(&self) -> usize {
        lock(&self.state).oidc_cancelled
    }

    /// Records the call, then runs `answer` against the state when the future is polled,
    /// after any hold on `op` was released.
    fn call<T: Send + 'static>(
        &self,
        op: FakeOp,
        call: FakeCall,
        answer: impl FnOnce(&mut State) -> Result<T, HubError> + Send + 'static,
    ) -> HubFuture<T> {
        let hold = {
            let mut state = lock(&self.state);
            state.calls.push(call);
            let index = state.holds.iter().position(|(held, _)| *held == op);
            index.map(|index| state.holds.remove(index).1)
        };
        let shared = Arc::clone(&self.state);
        async move {
            if let Some(hold) = hold {
                let _ = hold.await;
            }
            let mut state = lock(&shared);
            let injected = state
                .failures
                .iter()
                .position(|(failing, _)| *failing == op)
                .and_then(|index| state.failures.remove(index));
            match injected {
                Some((_, error)) => Err(error),
                None => answer(&mut state),
            }
        }
        .boxed()
    }
}

impl FakeCall {
    fn op(&self) -> Option<FakeOp> {
        Some(match self {
            Self::Probe(_) => FakeOp::Probe,
            Self::Connect(_) => FakeOp::Connect,
            Self::Password { .. } => FakeOp::Password,
            Self::Oidc { .. } => FakeOp::Oidc,
            Self::SignOut(_) => FakeOp::SignOut,
            Self::Forget(_) | Self::Terminal(..) => return None,
            Self::Account(_) => FakeOp::Account,
            Self::Nodes(..) => FakeOp::Nodes,
            Self::Sessions(..) => FakeOp::Sessions,
            Self::End(..) => FakeOp::End,
        })
    }
}

impl State {
    fn unreachable(address: &HubAddress) -> HubError {
        HubError::Unreachable {
            address: address.clone(),
            cause: SharedCause::new(FakeOffline),
        }
    }

    /// The server a call reaches, or why it cannot be reached.
    fn reach(&self, address: &HubAddress) -> Result<&FakeServer, HubError> {
        self.servers
            .get(address)
            .filter(|server| server.online)
            .ok_or_else(|| Self::unreachable(address))
    }

    /// Checks the certificate against the decided trust, as the real adapter does.
    fn verify(&self, endpoint: &HubEndpointSpec) -> Result<&FakeServer, HubError> {
        let server = self.reach(&endpoint.address)?;
        match endpoint.trust {
            TrustPolicy::Pinned(pinned) if pinned != server.certificate => {
                Err(HubError::CertificateChanged {
                    address: endpoint.address.clone(),
                    pinned,
                    presented: server.certificate,
                })
            }
            TrustPolicy::PublicCa if !server.publicly_trusted => {
                Err(HubError::CertificateUntrusted {
                    address: endpoint.address.clone(),
                    presented: server.certificate,
                })
            }
            _ => Ok(server),
        }
    }

    /// The signed-in user of a hub; a revoked device loses its credentials.
    fn authenticate(&mut self, hub: &HubId) -> Result<&FakeServer, HubError> {
        if self.revoked.remove(hub) {
            self.credentials.remove(hub);
            return Err(HubError::Revoked);
        }
        if !self.credentials.contains_key(hub) {
            return Err(HubError::NotSignedIn);
        }
        self.reach(hub.address())
    }

    fn account_of(&self, hub: &HubId, username: &str) -> Result<Account, HubError> {
        let server = self.reach(hub.address())?;
        server
            .users
            .iter()
            .find(|user| user.username == username)
            .map(|user| user.account.clone())
            .ok_or(HubError::InvalidCredentials)
    }
}

impl HubTrust for FakeHub {
    fn probe(&self, address: &HubAddress) -> HubFuture<ProbedHub> {
        let address = address.clone();
        self.call(
            FakeOp::Probe,
            FakeCall::Probe(address.clone()),
            move |state| {
                let server = state.reach(&address)?;
                Ok(ProbedHub {
                    fingerprint: server.certificate,
                    publicly_trusted: server.publicly_trusted,
                })
            },
        )
    }
}

impl HubSignIn for FakeHub {
    fn connect(&self, endpoint: HubEndpointSpec) -> HubFuture<HubInfo> {
        let hub = HubId::from(endpoint.address.clone());
        self.call(
            FakeOp::Connect,
            FakeCall::Connect(hub.clone()),
            move |state| {
                let server = state.verify(&endpoint)?;
                if server.protocol != CLIENT_PROTOCOL {
                    return Err(HubError::UpgradeRequired {
                        hub: server.protocol,
                        client: CLIENT_PROTOCOL,
                    });
                }
                if state.revoked.remove(&hub) {
                    state.credentials.remove(&hub);
                    return Err(HubError::Revoked);
                }
                let signed_in = state.credentials.contains_key(&hub);
                let server = state.verify(&endpoint)?;
                Ok(server.info(signed_in))
            },
        )
    }

    fn password(
        &self,
        endpoint: HubEndpointSpec,
        username: String,
        password: Secret,
    ) -> HubFuture<Account> {
        let hub = HubId::from(endpoint.address.clone());
        let call = FakeCall::Password {
            hub: hub.clone(),
            username: username.clone(),
        };
        self.call(FakeOp::Password, call, move |state| {
            let server = state.verify(&endpoint)?;
            if !server.password_login {
                return Err(HubError::MethodUnavailable);
            }
            let known = server.users.iter().any(|user| {
                user.username == username && user.password.as_str() == password.expose()
            });
            if !known {
                return Err(HubError::InvalidCredentials);
            }
            state.revoked.remove(&hub);
            state.credentials.insert(hub.clone(), username.clone());
            state.account_of(&hub, &username)
        })
    }

    fn oidc(&self, endpoint: HubEndpointSpec, provider: String) -> OidcSignIn {
        let hub = HubId::from(endpoint.address.clone());
        let call = FakeCall::Oidc {
            hub: hub.clone(),
            provider: provider.clone(),
        };
        let (browser_tx, browser) = oneshot::channel();
        let (finish, finished) = oneshot::channel();
        let shared = Arc::clone(&self.state);
        let ready = {
            let provider = provider.clone();
            self.call(FakeOp::Oidc, call, move |state| {
                let server = state.verify(&endpoint)?;
                if !server
                    .oidc_providers
                    .iter()
                    .any(|known| known.id == provider)
                {
                    return Err(HubError::MethodUnavailable);
                }
                state.oidc = Some(finish);
                let _ = browser_tx.send(format!("https://idp.fake/authorize?provider={provider}"));
                Ok(())
            })
        };
        let done = async move {
            let mut guard = CancelGuard {
                state: Arc::clone(&shared),
                finished: false,
            };
            ready.await?;
            let username = finished.await.map_err(|_| HubError::SignInCancelled)??;
            guard.finished = true;
            let mut state = lock(&shared);
            state.revoked.remove(&hub);
            state.credentials.insert(hub.clone(), username.clone());
            state.account_of(&hub, &username)
        }
        .boxed();
        OidcSignIn { browser, done }
    }

    fn sign_out(&self, hub: &HubId) -> HubFuture<()> {
        let hub = hub.clone();
        self.call(
            FakeOp::SignOut,
            FakeCall::SignOut(hub.clone()),
            move |state| {
                state.credentials.remove(&hub);
                Ok(())
            },
        )
    }

    fn forget(&self, hub: &HubId) -> Result<(), HubError> {
        let mut state = lock(&self.state);
        state.calls.push(FakeCall::Forget(hub.clone()));
        state.credentials.remove(hub);
        Ok(())
    }
}

/// Counts a browser sign-in that was dropped before it finished.
struct CancelGuard {
    state: Arc<Mutex<State>>,
    finished: bool,
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        if !self.finished {
            let mut state = lock(&self.state);
            state.oidc = None;
            state.oidc_cancelled += 1;
        }
    }
}

impl HubSessions for FakeHub {
    fn account(&self, hub: &HubId) -> HubFuture<Account> {
        let hub = hub.clone();
        self.call(
            FakeOp::Account,
            FakeCall::Account(hub.clone()),
            move |state| {
                state.authenticate(&hub)?;
                let username = state.credentials[&hub].clone();
                state.account_of(&hub, &username)
            },
        )
    }

    fn nodes(&self, hub: &HubId, org: &OrgId) -> HubFuture<Vec<NodeSummary>> {
        let (hub, org) = (hub.clone(), org.clone());
        self.call(
            FakeOp::Nodes,
            FakeCall::Nodes(hub.clone(), org),
            move |state| Ok(state.authenticate(&hub)?.nodes.clone()),
        )
    }

    fn sessions(&self, hub: &HubId, org: &OrgId) -> HubFuture<Vec<HubSessionSummary>> {
        let (hub, org) = (hub.clone(), org.clone());
        self.call(
            FakeOp::Sessions,
            FakeCall::Sessions(hub.clone(), org.clone()),
            move |state| {
                Ok(state
                    .authenticate(&hub)?
                    .sessions
                    .iter()
                    .filter(|session| session.org == org)
                    .filter(|session| {
                        matches!(
                            session.state,
                            HubSessionState::Starting | HubSessionState::Running
                        )
                    })
                    .cloned()
                    .collect())
            },
        )
    }

    fn end(&self, hub: &HubId, session: &HubSessionId) -> HubFuture<()> {
        let (hub, session) = (hub.clone(), session.clone());
        self.call(
            FakeOp::End,
            FakeCall::End(hub.clone(), session.clone()),
            move |state| {
                state.authenticate(&hub)?;
                let server = state
                    .servers
                    .get_mut(hub.address())
                    .ok_or(HubError::NotFound)?;
                let found = server
                    .sessions
                    .iter_mut()
                    .find(|candidate| candidate.id == session)
                    .ok_or(HubError::NotFound)?;
                found.state = HubSessionState::Exited;
                Ok(())
            },
        )
    }

    fn terminal(&self, hub: &HubId, target: HubTerminalTarget) -> Arc<dyn Transport> {
        lock(&self.state)
            .calls
            .push(FakeCall::Terminal(hub.clone(), target));
        Arc::new(self.terminals.clone())
    }
}

#[cfg(test)]
mod tests;
