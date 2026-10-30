//! An in-memory hub for tests: one fake behind the access ports.
//!
//! Enable `test-support` in a consumer's dev-dependencies. The fake does no I/O and keeps its
//! "hubs" in memory. A call completes when it is polled; [`FakeHub::hold`] makes the next call
//! of one kind wait until the test releases it, so a test can observe the state in between.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::FutureExt as _;
use futures::channel::oneshot;
use futures::future::BoxFuture;
use nocterm_core::error::SharedCause;
use nocterm_session::Secret;

use crate::address::{HubAddress, HubId};
use crate::error::HubError;
use crate::ports::{HubFuture, HubSignIn, HubTrust, OidcSignIn};
use crate::records::{Account, HubEndpointSpec, HubInfo, KeyStatus, SignedIn};
use crate::trust::{ProbedHub, TrustPolicy};

mod admin_keys;
mod agent_access;
mod devices;
mod local_source;
mod login;
mod recovery;
mod server;
pub use local_source::ScriptedLocalAccountFactory;

pub use admin_keys::FakeAdminKeys;
pub use agent_access::FakeAgentAccess;
pub use devices::FakeAccountDevices;
pub use server::{FakeKeyset, FakeServer, FakeUser};

/// The protocol version the fake client speaks.
pub const CLIENT_PROTOCOL: u32 = 1;

/// The kinds of call a test can observe, hold or make fail.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FakeOp {
    Probe,
    Connect,
    Password,
    Oidc,
    SetVaultPassphrase,
    Recover,
    ReissueRecoveryKey,
    SignOut,
    Forget,
}

/// One call the fake received. Secrets are not recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FakeCall {
    Probe(HubAddress),
    Connect(HubId),
    Password { hub: HubId, username: String },
    Oidc { hub: HubId, provider: String },
    SetVaultPassphrase(HubId),
    Recover(HubId),
    ReissueRecoveryKey(HubId),
    SignOut(HubId),
    Forget(HubId),
}

/// Why a fake hub did not answer.
#[derive(Debug, thiserror::Error)]
#[error("the fake hub is offline")]
pub struct FakeOffline;

/// An in-memory hub implementing [`HubTrust`] and [`HubSignIn`].
///
/// Clones share one state, so a test keeps a handle while a service owns another.
#[derive(Clone, Default)]
pub struct FakeHub {
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    servers: BTreeMap<HubAddress, FakeServer>,
    receipts: recovery::Receipts,
    /// Hubs this device holds credentials for, and who it is signed in as.
    credentials: BTreeMap<HubId, String>,
    revoked: BTreeSet<HubId>,
    calls: Vec<FakeCall>,
    failures: VecDeque<(FakeOp, HubError)>,
    holds: Vec<(FakeOp, oneshot::Receiver<()>)>,
    oidc: Option<oneshot::Sender<Result<String, HubError>>>,
    oidc_cancelled: usize,
    /// Keys a sign-in could not open, waiting for the recovery key, and what should open them
    /// afterwards.
    recovering: BTreeMap<HubId, String>,
    /// Hubs whose recovery key was shown on this device and not confirmed as saved.
    unconfirmed: BTreeSet<HubId>,
    /// Hubs whose account keys are closed, as after a restart until the vault is unlocked.
    locked: BTreeSet<HubId>,
    /// How many times the keys of any hub were opened or dropped, and who waits for the next.
    key_changes: u64,
    key_waiters: Vec<oneshot::Sender<()>>,
}

impl State {
    fn discard(&mut self, hub: &HubId) -> Result<(), HubError> {
        self.calls.push(FakeCall::Forget(hub.clone()));
        let injected = self
            .failures
            .iter()
            .position(|(op, _)| *op == FakeOp::Forget)
            .and_then(|index| self.failures.remove(index));
        if let Some((_, error)) = injected {
            return Err(error);
        }
        self.credentials.remove(hub);
        self.recovering.remove(hub);
        self.unconfirmed.remove(hub);
        self.receipts.begin(hub);
        self.keys_changed();
        Ok(())
    }
    fn keys_changed(&mut self) {
        self.key_changes += 1;
        for waiter in self.key_waiters.drain(..) {
            let _ = waiter.send(());
        }
    }
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
        let mut state = lock(&self.state);
        state.receipts.begin(hub);
        state.credentials.insert(hub.clone(), username.to_owned());
    }

    /// Makes the hub revoke this device: its next call finds the device revoked.
    /// Ends the sign-in of this device as an expired login does: the credentials are gone, the
    /// account keys it holds are not.
    pub fn expire_login(&self, hub: &HubId) {
        lock(&self.state).credentials.remove(hub);
    }

    /// Marks the recovery key shown on this device for `hub` as never confirmed, as a dialog
    /// closed on that step leaves it.
    pub fn leave_recovery_key_unconfirmed(&self, hub: &HubId) {
        lock(&self.state).unconfirmed.insert(hub.clone());
    }

    /// Closes the account keys of `hub`, as a restart does until the vault is unlocked.
    pub fn lock_keys(&self, hub: &HubId) {
        let mut state = lock(&self.state);
        state.receipts.rotate(hub);
        if state.locked.insert(hub.clone()) {
            state.keys_changed();
        }
    }

    /// Opens the account keys of `hub`, as unlocking the vault does.
    pub fn unlock_keys(&self, hub: &HubId) {
        let mut state = lock(&self.state);
        state.receipts.rotate(hub);
        if state.locked.remove(hub) {
            state.keys_changed();
        }
    }

    /// Tells whoever follows the keys that something that depends on them changed.
    pub(crate) fn raise_keys_changed(&self) {
        lock(&self.state).keys_changed();
    }

    pub fn revoke_device(&self, hub: &HubId) {
        lock(&self.state).revoked.insert(hub.clone());
    }

    pub fn is_signed_in(&self, hub: &HubId) -> bool {
        lock(&self.state).credentials.contains_key(hub)
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
            .filter(|call| call.op() == op)
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
    fn op(&self) -> FakeOp {
        match self {
            Self::Probe(_) => FakeOp::Probe,
            Self::Connect(_) => FakeOp::Connect,
            Self::Password { .. } => FakeOp::Password,
            Self::Oidc { .. } => FakeOp::Oidc,
            Self::SetVaultPassphrase(_) => FakeOp::SetVaultPassphrase,
            Self::Recover(_) => FakeOp::Recover,
            Self::ReissueRecoveryKey(_) => FakeOp::ReissueRecoveryKey,
            Self::SignOut(_) => FakeOp::SignOut,
            Self::Forget(_) => FakeOp::Forget,
        }
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

    /// What a sign-in with `secret` does to the keys of `username`: opens them, makes them (the
    /// recovery key is `recovery-<username>`) or waits for the recovery key.
    fn open_keys(
        &mut self,
        hub: &HubId,
        username: &str,
        secret: &str,
        shared: std::sync::Weak<Mutex<State>>,
    ) -> Result<KeyStatus, HubError> {
        let server = self
            .servers
            .get_mut(hub.address())
            .filter(|server| server.online)
            .ok_or_else(|| Self::unreachable(hub.address()))?;
        if !server.keyed {
            return Ok(KeyStatus::Open);
        }
        match server.keysets.get(username) {
            None => {
                let recovery = format!("recovery-{username}");
                server.keysets.insert(
                    username.to_owned(),
                    FakeKeyset {
                        secret: secret.to_owned(),
                        recovery: recovery.clone(),
                    },
                );
                self.unconfirmed.insert(hub.clone());
                Ok(KeyStatus::Created(
                    recovery::Recipient::capture(shared, self, hub).offer(Secret::new(recovery)),
                ))
            }
            Some(keyset) if keyset.secret == secret => Ok(if self.unconfirmed.contains(hub) {
                KeyStatus::RecoveryKeyUnconfirmed(
                    recovery::Recipient::capture(shared, self, hub).issuer(),
                )
            } else {
                KeyStatus::Open
            }),
            Some(_) => {
                self.recovering.insert(hub.clone(), secret.to_owned());
                Ok(KeyStatus::RecoveryNeeded)
            }
        }
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
    ) -> HubFuture<SignedIn> {
        let hub = HubId::from(endpoint.address.clone());
        let call = FakeCall::Password {
            hub: hub.clone(),
            username: username.clone(),
        };
        let shared = Arc::downgrade(&self.state);
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
            state.receipts.begin(&hub);
            state.credentials.insert(hub.clone(), username.clone());
            let account = state.account_of(&hub, &username)?;
            let keys = state.open_keys(&hub, &username, password.expose(), shared.clone())?;
            let login = login::receipt(shared.clone(), state, &hub);
            Ok(SignedIn {
                account,
                keys,
                login,
            })
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
            state.receipts.begin(&hub);
            state.credentials.insert(hub.clone(), username.clone());
            state.recovering.remove(&hub);
            let account = state.account_of(&hub, &username)?;
            let keys = if state.reach(hub.address())?.keyed {
                KeyStatus::PassphraseNeeded
            } else {
                KeyStatus::Open
            };
            let login = login::receipt(Arc::downgrade(&shared), &mut state, &hub);
            Ok(SignedIn {
                account,
                keys,
                login,
            })
        }
        .boxed();
        OidcSignIn {
            browser: Box::pin(async move { browser.await.map_err(|_| HubError::SignInCancelled) }),
            done,
        }
    }

    fn set_vault_passphrase(&self, hub: &HubId, passphrase: Secret) -> HubFuture<KeyStatus> {
        let hub = hub.clone();
        let shared = Arc::downgrade(&self.state);
        self.call(
            FakeOp::SetVaultPassphrase,
            FakeCall::SetVaultPassphrase(hub.clone()),
            move |state| {
                state.authenticate(&hub)?;
                let username = state.credentials[&hub].clone();
                state.open_keys(&hub, &username, passphrase.expose(), shared)
            },
        )
    }

    fn recover(&self, hub: &HubId, recovery_key: Secret) -> HubFuture<()> {
        let hub = hub.clone();
        self.call(
            FakeOp::Recover,
            FakeCall::Recover(hub.clone()),
            move |state| {
                state.authenticate(&hub)?;
                let username = state.credentials[&hub].clone();
                let secret = state
                    .recovering
                    .get(&hub)
                    .cloned()
                    .ok_or(HubError::NoRecoveryPending)?;
                let keyset = state
                    .servers
                    .get_mut(hub.address())
                    .and_then(|server| server.keysets.get_mut(&username))
                    .ok_or(HubError::NoRecoveryPending)?;
                if keyset.recovery != recovery_key.expose() {
                    return Err(HubError::RecoveryKeyInvalid);
                }
                keyset.secret = secret;
                state.recovering.remove(&hub);
                Ok(())
            },
        )
    }

    fn recovery_key_unconfirmed(&self, hub: &HubId) -> bool {
        let state = lock(&self.state);
        state.unconfirmed.contains(hub)
            && !state.recovering.contains_key(hub)
            && !state.locked.contains(hub)
    }

    fn keys_revision(&self) -> u64 {
        lock(&self.state).key_changes
    }

    fn keys_changed(&self, seen: u64) -> BoxFuture<'static, u64> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            loop {
                let waiting = {
                    let mut state = lock(&state);
                    if state.key_changes != seen {
                        return state.key_changes;
                    }
                    let (sender, receiver) = oneshot::channel();
                    state.key_waiters.push(sender);
                    receiver
                };
                if waiting.await.is_err() {
                    return lock(&state).key_changes;
                }
            }
        })
    }

    fn recovery_key_offer(&self, hub: &HubId) -> Result<crate::RecoveryKeyReissue, HubError> {
        let mut state = lock(&self.state);
        if state.locked.contains(hub) || state.recovering.contains_key(hub) {
            return Err(HubError::KeysLocked);
        }
        Ok(recovery::Recipient::capture(Arc::downgrade(&self.state), &mut state, hub).issuer())
    }

    fn sign_out(&self, hub: &HubId) -> HubFuture<()> {
        let hub = hub.clone();
        self.call(
            FakeOp::SignOut,
            FakeCall::SignOut(hub.clone()),
            move |state| {
                state.receipts.begin(&hub);
                state.credentials.remove(&hub);
                state.recovering.remove(&hub);
                state.unconfirmed.remove(&hub);
                Ok(())
            },
        )
    }

    fn forget(&self, hub: &HubId) -> Result<(), HubError> {
        lock(&self.state).discard(hub)
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

#[cfg(test)]
mod tests;
