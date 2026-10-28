//! Confirming the keys of organization administrators.
//!
//! A hub tells which account granted an organization's collection key and publishes that
//! account's public key, so it could substitute its own. This device therefore opens a grant
//! only under a key whose fingerprint a person compared with the administrator's own (the one
//! `hubctl` shows). [`HubAdminKeys`] is how the application lists the keys waiting for that
//! comparison and records the decision.

/// An organization administrator whose key this device has not confirmed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingAdminKey {
    /// What the user calls the organization.
    pub organization: String,
    /// The administrator's name, as the hub lists it.
    pub admin: String,
    /// The administrator's account, the argument of [`HubAdminKeys::confirm_admin_key`].
    pub account_id: String,
    /// The fingerprint of the key the hub offers, to compare with the administrator's own.
    pub fingerprint: String,
    /// The fingerprint confirmed earlier when the offered key is not the one that was. A changed
    /// key is either a new key of the administrator or an attack; confirm it only out of band.
    pub replaces: Option<String>,
}

/// Why a pending key could not be confirmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AdminKeyError {
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
}

/// The administrator keys of a hub's account, waiting for a person to compare them.
pub trait HubAdminKeys: Send + Sync {
    /// The administrators of the account's organizations whose keys wait for confirmation, as
    /// the last look at the hub found them. Empty while the account keys are closed.
    fn pending_admin_keys(&self, hub: &crate::HubId) -> Vec<PendingAdminKey>;

    /// Confirms the key of `account_id` that was shown with `fingerprint`, after a person
    /// compared it with the administrator's own. The groups it unlocks are read again promptly,
    /// and [`HubSignIn::keys_changed`](crate::HubSignIn::keys_changed) resolves.
    fn confirm_admin_key(
        &self,
        hub: &crate::HubId,
        account_id: &str,
        fingerprint: &str,
    ) -> Result<(), AdminKeyError>;
}
