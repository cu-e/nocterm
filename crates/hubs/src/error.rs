//! Typed failures of signing in to a hub and of managing known hubs.

use nocterm_hubs_model::{FingerprintError, HubAddressError, HubError, HubId};
use nocterm_store::StoreError;

/// Why signing in to a hub did not complete.
#[derive(Clone, Debug, thiserror::Error)]
pub enum SignInError {
    #[error("the server address is not valid")]
    Address(#[from] HubAddressError),
    #[error("the fingerprint is not valid")]
    Fingerprint(#[from] FingerprintError),
    #[error("the hub could not be used")]
    Hub(#[from] HubError),
    /// The address is a server that is listed already. Signing in again to it goes through its
    /// row, and a server whose certificate changed has to be removed first.
    #[error("the server {0} is listed already")]
    AlreadyKnown(HubId),
    /// The hub has no owner yet; this client cannot set it up.
    #[error("the hub is not set up yet")]
    NotInitialized,
    /// The sign-in worked but the server could not be added to the settings, so the
    /// credentials of this device were erased again.
    #[error("the server could not be added to the settings")]
    Save(#[source] StoreError),
}

/// Why forgetting or signing out of a known hub failed.
#[derive(Clone, Debug, thiserror::Error)]
pub enum HubsError {
    #[error("no such hub: {0}")]
    Unknown(HubId),
    #[error("the hub could not be used")]
    Hub(#[from] HubError),
    #[error("the settings could not be saved")]
    Save(#[source] StoreError),
}
