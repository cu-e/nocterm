//! An in-memory [`ChatSshCertifier`] that counts sweeps per hub (ADR 0147).
use crate::{ChatSshCertifier, CloudAgentError, CloudFuture};
use futures::FutureExt as _;
use nocterm_hubs_model::HubId;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

#[derive(Default)]
struct State {
    sweeps: BTreeMap<HubId, usize>,
    forgotten: Vec<HubId>,
    failing: BTreeMap<HubId, fn() -> CloudAgentError>,
}

/// Counts the sweeps of each hub; clones share one state.
#[derive(Clone, Default)]
pub struct FakeChatSshCertifier {
    state: Arc<Mutex<State>>,
}
impl FakeChatSshCertifier {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// Sweeps of `hub` fail with what `error` makes until [`Self::heal`].
    pub fn fail(&self, hub: &HubId, error: fn() -> CloudAgentError) {
        self.state().failing.insert(hub.clone(), error);
    }
    pub fn heal(&self, hub: &HubId) {
        self.state().failing.remove(hub);
    }
    /// How many sweeps of `hub` succeeded.
    pub fn sweeps(&self, hub: &HubId) -> usize {
        self.state().sweeps.get(hub).copied().unwrap_or(0)
    }
    /// The hubs whose bound chats are no longer read, in order.
    pub fn forgotten(&self) -> Vec<HubId> {
        self.state().forgotten.clone()
    }
}
impl ChatSshCertifier for FakeChatSshCertifier {
    fn sweep(&self, hub: &HubId) -> CloudFuture<usize> {
        let mut state = self.state();
        let result = match state.failing.get(hub) {
            Some(error) => Err(error()),
            None => {
                *state.sweeps.entry(hub.clone()).or_default() += 1;
                Ok(0)
            }
        };
        async move { result }.boxed()
    }
    fn forget(&self, hub: &HubId) {
        self.state().forgotten.push(hub.clone());
    }
}
