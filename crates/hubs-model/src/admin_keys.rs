//! Confirming the keys of organization administrators.
//!
//! A hub tells which account granted an organization's collection key and publishes that
//! account's public key, so it could substitute its own. This device therefore opens a grant
//! only under a key whose fingerprint a person compared with the administrator's own (the one
//! `hubctl` shows). [`HubAdminKeys`] is how the application lists the keys waiting for that
//! comparison and records the decision.

use std::{fmt, sync::Arc};

/// An organization administrator whose key this device has not confirmed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingAdminKey {
    /// What the user calls the organization.
    pub organization: String,
    /// The administrator's name, as the hub lists it.
    pub admin: String,
    /// The administrator's account, whose key is being compared.
    pub account_id: String,
    /// The fingerprint of the key the hub offers, to compare with the administrator's own.
    pub fingerprint: String,
    /// The fingerprint confirmed earlier when the offered key is not the one that was. A changed
    /// key is either a new key of the administrator or an attack; confirm it only out of band.
    pub replaces: Option<String>,
}

/// Why a pending key could not be confirmed.
#[derive(Clone, Debug, thiserror::Error)]
pub enum AdminKeyError {
    /// The original recipient or displayed action is no longer current.
    #[error("the displayed administrator key belongs to an earlier account")]
    Stale,
    /// The account keys are closed on this device.
    #[error("the account keys are not open on this device")]
    Locked,
    /// Nothing is pending for the account: the list changed since it was shown.
    #[error("no key of that administrator is waiting for confirmation")]
    NotPending,
    /// The fingerprint is not the one of the key now on offer: the hub offers another key than
    /// the one that was shown.
    #[error("the fingerprint is not the one of the key the hub now offers")]
    Mismatch,
    /// The confirmation could not be published durably.
    #[error("the administrator key confirmation cannot be kept")]
    Persistence {
        #[source]
        cause: nocterm_core::error::SharedCause,
    },
}

/// Completion of an administrator-key operation, including durable publication.
pub type AdminKeyFuture<T> = futures::future::BoxFuture<'static, Result<T, AdminKeyError>>;

/// Authority supplied by the native adapter for one original recipient and offer.
pub trait AdminKeyConfirmation: Send + Sync {
    fn confirm(&self, fingerprint: &str) -> AdminKeyFuture<()>;
}

/// A displayed key and its original recipient capability.
#[derive(Clone)]
pub struct AdminKeyOffer {
    hub: crate::HubId,
    key: PendingAdminKey,
    confirmation: Arc<dyn AdminKeyConfirmation>,
}
impl AdminKeyOffer {
    pub fn new(
        hub: crate::HubId,
        key: PendingAdminKey,
        confirmation: Arc<dyn AdminKeyConfirmation>,
    ) -> Self {
        Self {
            hub,
            key,
            confirmation,
        }
    }
    pub fn hub(&self) -> &crate::HubId {
        &self.hub
    }
    pub fn key(&self) -> &PendingAdminKey {
        &self.key
    }
    pub fn confirm(&self, fingerprint: &str) -> AdminKeyFuture<()> {
        self.confirmation.confirm(fingerprint)
    }
}
impl fmt::Debug for AdminKeyOffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdminKeyOffer")
            .field("hub", &self.hub)
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}
impl PartialEq for AdminKeyOffer {
    fn eq(&self, other: &Self) -> bool {
        self.hub == other.hub
            && self.key == other.key
            && Arc::ptr_eq(&self.confirmation, &other.confirmation)
    }
}

/// Offers retain their original recipient through delayed confirmation callbacks.
pub trait HubAdminKeys: Send + Sync {
    fn pending_admin_keys(&self, hub: &crate::HubId) -> Vec<AdminKeyOffer>;
}
