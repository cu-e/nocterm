//! The administrator keys of a fake hub account, for tests of what shows and confirms them.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::address::HubId;
use crate::admin_keys::{AdminKeyError, HubAdminKeys, PendingAdminKey};
use crate::testing::FakeHub;

/// An in-memory [`HubAdminKeys`]. Clones share one list, so a test keeps a handle while a
/// service owns another.
///
/// It lives beside a [`FakeHub`]: offering or confirming a key raises that hub's keys notice
/// ([`crate::HubSignIn::keys_changed`]) as the adapter does, and a hub whose keys are closed
/// ([`FakeHub::lock_keys`]) lists nothing and confirms nothing.
#[derive(Clone)]
pub struct FakeAdminKeys {
    hub: FakeHub,
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    pending: Vec<(HubId, PendingAdminKey)>,
    confirmed: Vec<(HubId, PendingAdminKey)>,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A fingerprint without the grouping a person may retype it with.
fn squash(fingerprint: &str) -> String {
    fingerprint
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

impl FakeAdminKeys {
    /// Keys of the accounts of `hub`'s servers, noticed through its keys notice.
    pub fn alongside(hub: &FakeHub) -> Self {
        Self {
            hub: hub.clone(),
            state: Arc::default(),
        }
    }

    /// A synchronization found `key` unconfirmed in the account on `hub`: it replaces the
    /// entry of the same administrator and organization.
    pub fn offer(&self, hub: &HubId, key: PendingAdminKey) {
        {
            let mut state = lock(&self.state);
            state.pending.retain(|(known, pending)| {
                known != hub
                    || pending.account_id != key.account_id
                    || pending.organization != key.organization
            });
            state.pending.push((hub.clone(), key));
        }
        self.hub.raise_keys_changed();
    }

    /// The keys confirmed so far, in order, with the hub they belong to.
    pub fn confirmed(&self) -> Vec<(HubId, PendingAdminKey)> {
        lock(&self.state).confirmed.clone()
    }
}

impl HubAdminKeys for FakeAdminKeys {
    fn pending_admin_keys(&self, hub: &HubId) -> Vec<PendingAdminKey> {
        if self.hub.keys_closed(hub) {
            return Vec::new();
        }
        lock(&self.state)
            .pending
            .iter()
            .filter(|(known, _)| known == hub)
            .map(|(_, key)| key.clone())
            .collect()
    }

    fn confirm_admin_key(
        &self,
        hub: &HubId,
        account_id: &str,
        fingerprint: &str,
    ) -> Result<(), AdminKeyError> {
        if self.hub.keys_closed(hub) {
            return Err(AdminKeyError::Locked);
        }
        {
            let mut state = lock(&self.state);
            let shown = state
                .pending
                .iter()
                .position(|(known, key)| known == hub && key.account_id == account_id)
                .ok_or(AdminKeyError::NotPending)?;
            if squash(&state.pending[shown].1.fingerprint) != squash(fingerprint) {
                return Err(AdminKeyError::Mismatch);
            }
            let (_, key) = state.pending.remove(shown);
            state.confirmed.push((hub.clone(), key));
        }
        self.hub.raise_keys_changed();
        Ok(())
    }
}
