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
//! the handle the application uses to read the list and confirm an entry. The pins are written
//! to the account cache, so they outlive a restart and are erased with the account.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nocterm_keys::{PinError, PinnedKeys, PublicKey, TrustedKey};

use super::Slot;

/// A group's administrator whose key this device has not confirmed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingAdminKey {
    /// What the user calls the organization.
    pub organization: String,
    /// The administrator's account, the argument of [`AdminKeys::confirm`].
    pub account_id: String,
    /// The administrator's name, as the hub lists it.
    pub admin: String,
    /// The fingerprint of the key the hub offers, to compare with the administrator's own.
    pub fingerprint: String,
    /// The fingerprint pinned earlier when the key is not the one that was pinned. A changed
    /// key is either a new key of the administrator or an attack; confirm it only out of band.
    pub replaces: Option<String>,
}

/// Why a pending key could not be confirmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AdminKeyError {
    /// The keys are closed on this device.
    #[error("the account keys are not open on this device")]
    Locked,
    /// Nothing is pending for the account: the list changed since it was shown.
    #[error("no key of that administrator is waiting for confirmation")]
    NotPending,
    /// The fingerprint is not the one of the key now on offer: the hub offers another key than
    /// the one that was shown.
    #[error("the fingerprint is not the one of the key the hub now offers")]
    Mismatch,
}

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
    /// hub, in place of the ones noted for that kind before.
    pub(super) fn replace_pending(
        &mut self,
        kind: &str,
        pending: Vec<(PendingAdminKey, PublicKey)>,
    ) {
        self.pending.retain(|p| p.kind != kind);
        self.pending
            .extend(pending.into_iter().map(|(view, key)| Pending {
                kind: kind.to_owned(),
                view,
                key,
            }));
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
    /// the administrator's. The groups it unlocks are read the next time the account is read.
    pub fn confirm(&self, account_id: &str, fingerprint: &str) -> Result<(), AdminKeyError> {
        let keys = self.slot.open().ok_or(AdminKeyError::Locked)?;
        let pinned = {
            let mut admins = keys.admins();
            admins.confirm(account_id, fingerprint)?;
            admins.pinned().clone()
        };
        self.slot.remember_pins(&pinned);
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
