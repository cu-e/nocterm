//! The public keys of the administrators whose grants this device accepts.
//!
//! A hub tells which account granted a group's collection key and publishes that account's
//! public key, so it could also substitute its own. Trust is therefore first use with a
//! fingerprint: a grant is opened only with a key this device pinned after a person compared
//! its fingerprint with the administrator's (`hubctl orgs confirm` shows the same one). A group
//! whose granter is not pinned is left out and reported as pending, with the fingerprint to
//! compare; a key that differs from the pinned one is reported the same way, marked as a
//! change, and replaces the pin only when confirmed again.
//!
//! [`AdminPins`] holds the pins and the pending list of one set of open keys; [`AdminKeys`] is
//! the handle the application uses to read the list and confirm an entry. The pins are public
//! keys, written to the account cache in the clear: they outlive a restart and are erased with
//! the account. The keys notice is raised when the pending list changes and when an entry is
//! confirmed, so the replica and the vault read the groups again and the views follow.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nocterm_keys::{PinError, PinnedKeys, PublicKey, TrustedKey};

use super::Slot;

pub use nocterm_hubs_model::{AdminKeyError, PendingAdminKey};

struct Pending {
    /// The kind of collection the grant was met in; each kind is looked at on its own.
    kind: String,
    view: PendingAdminKey,
    key: PublicKey,
}

/// The pins and the pending list of one set of open keys.
pub(crate) struct AdminPins {
    pinned: PinnedKeys,
    pending: Vec<Pending>,
}

impl AdminPins {
    pub(super) fn new(pinned: PinnedKeys) -> Self {
        Self {
            pinned,
            pending: Vec::new(),
        }
    }

    pub(super) fn pinned(&self) -> &PinnedKeys {
        &self.pinned
    }

    pub(super) fn trust(
        &self,
        account_id: &str,
        offered: &PublicKey,
    ) -> Result<TrustedKey, PinError> {
        self.pinned.trust(account_id, offered)
    }

    /// Records the groups of `kind` whose granter was met unpinned in the last look at the
    /// hub, in place of the ones noted for that kind before. Whether the list a person sees
    /// changed.
    pub(super) fn replace_pending(
        &mut self,
        kind: &str,
        pending: Vec<(PendingAdminKey, PublicKey)>,
    ) -> bool {
        let before = self.pending();
        self.pending.retain(|p| p.kind != kind);
        self.pending
            .extend(pending.into_iter().map(|(view, key)| Pending {
                kind: kind.to_owned(),
                view,
                key,
            }));
        self.pending() != before
    }

    /// One entry per administrator and organization, however many kinds met the same grant.
    fn pending(&self) -> Vec<PendingAdminKey> {
        let mut views: Vec<PendingAdminKey> = Vec::new();
        for pending in &self.pending {
            if !views.contains(&pending.view) {
                views.push(pending.view.clone());
            }
        }
        views
    }

    /// Pins the key of `account_id` that was shown with `fingerprint`.
    fn confirm(&mut self, account_id: &str, fingerprint: &str) -> Result<(), AdminKeyError> {
        let shown = self
            .pending
            .iter()
            .find(|p| p.view.account_id == account_id)
            .ok_or(AdminKeyError::NotPending)?;
        if squash(&shown.view.fingerprint) != squash(fingerprint) {
            return Err(AdminKeyError::Mismatch);
        }
        let key = shown.key;
        if shown.view.replaces.is_some() {
            self.pinned.replace(account_id, key);
        } else if self.pinned.pin(account_id, key).is_err() {
            // Pinned meanwhile to another key: what was confirmed is not what is pinned.
            return Err(AdminKeyError::Mismatch);
        }
        self.pending.retain(|p| p.view.account_id != account_id);
        Ok(())
    }
}

/// A fingerprint without the grouping a person may retype it with.
fn squash(fingerprint: &str) -> String {
    fingerprint
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Reads and confirms the administrator keys of one hub's account.
#[derive(Clone)]
pub struct AdminKeys {
    slot: Arc<Slot>,
}

impl AdminKeys {
    pub(crate) fn new(slot: Arc<Slot>) -> Self {
        Self { slot }
    }

    /// The administrators waiting for confirmation, as the last look at the hub found them.
    /// Empty while the keys are closed.
    pub fn pending(&self) -> Vec<PendingAdminKey> {
        self.slot
            .open()
            .map(|keys| keys.admins().pending())
            .unwrap_or_default()
    }

    /// Pins the key shown with `fingerprint` for `account_id`, after a person compared it with
    /// the administrator's. The keys notice is raised, so the replica and the vault read the
    /// groups it unlocks at once.
    pub fn confirm(&self, account_id: &str, fingerprint: &str) -> Result<(), AdminKeyError> {
        let keys = self.slot.open().ok_or(AdminKeyError::Locked)?;
        let pinned = {
            let mut admins = keys.admins();
            admins.confirm(account_id, fingerprint)?;
            admins.pinned().clone()
        };
        self.slot.remember_pins(&pinned);
        self.slot.notice().raise();
        Ok(())
    }
}

/// The lock of the pins, shared by the keys and their handle.
pub(super) type Admins = Mutex<AdminPins>;

pub(super) fn lock(admins: &Admins) -> MutexGuard<'_, AdminPins> {
    admins.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
