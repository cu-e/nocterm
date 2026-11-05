//! The administrator keys of a fake hub account, for tests of what shows and confirms them.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::address::HubId;
use crate::admin_keys::{
    AdminKeyConfirmation, AdminKeyError, AdminKeyOffer, HubAdminKeys, PendingAdminKey,
};
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
    fn pending_admin_keys(&self, hub: &HubId) -> Vec<AdminKeyOffer> {
        let mut account = super::lock(&self.hub.state);
        if account.locked.contains(hub) {
            return Vec::new();
        }
        let recipient =
            super::recovery::Recipient::capture(Arc::downgrade(&self.hub.state), &mut account, hub);
        lock(&self.state)
            .pending
            .iter()
            .filter(|(known, _)| known == hub)
            .map(|(_, key)| {
                AdminKeyOffer::new(
                    hub.clone(),
                    key.clone(),
                    Arc::new(Confirmation {
                        recipient: recipient.clone(),
                        state: Arc::downgrade(&self.state),
                        hub: hub.clone(),
                        key: key.clone(),
                    }),
                )
            })
            .collect()
    }
}
#[derive(Clone)]
struct Confirmation {
    recipient: super::recovery::Recipient,
    state: std::sync::Weak<Mutex<State>>,
    hub: HubId,
    key: PendingAdminKey,
}
impl AdminKeyConfirmation for Confirmation {
    fn confirm(&self, fingerprint: &str) -> crate::AdminKeyFuture<()> {
        let original = self.clone();
        let fingerprint = fingerprint.to_owned();
        Box::pin(async move { original.confirm_current(&fingerprint) })
    }
}
impl Confirmation {
    fn confirm_current(&self, fingerprint: &str) -> Result<(), AdminKeyError> {
        let mut result = Err(AdminKeyError::Stale);
        self.recipient
            .with_current(|account| {
                let shared = self.state.upgrade().ok_or(crate::HubError::Stale)?;
                let mut state = lock(&shared);
                result = (|| {
                    let shown = state
                        .pending
                        .iter()
                        .position(|(hub, key)| hub == &self.hub && key == &self.key)
                        .ok_or(AdminKeyError::NotPending)?;
                    if squash(&self.key.fingerprint) != squash(fingerprint) {
                        return Err(AdminKeyError::Mismatch);
                    }
                    let (_, key) = state.pending.remove(shown);
                    state.confirmed.push((self.hub.clone(), key));
                    account.keys_changed();
                    Ok(())
                })();
                Ok(())
            })
            .map_err(|_| AdminKeyError::Stale)?;
        result
    }
}
