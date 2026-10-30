//! The devices of an account on a hub, as the Servers page lists and revokes them.

use crate::address::HubId;
use crate::ports::HubFuture;

/// One device signed in to the account on a hub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceSummary {
    /// The hub's identifier of the device.
    pub id: String,
    /// The name the device gave itself when it signed in.
    pub name: String,
    /// What kind of client it is, as the hub reports it (`desktop`, `browser`, `hubctl`, ...).
    pub platform: String,
    /// When the device first signed in, in milliseconds since the Unix epoch.
    pub created_at_ms: i64,
    /// When the hub last heard from the device, in milliseconds since the Unix epoch.
    pub last_seen_at_ms: i64,
    /// Whether this is the device asking.
    pub is_current: bool,
}

/// Lists and revokes the devices of the account this device is signed in to on a hub.
///
/// An implementation captures the original native membership before it returns the future
/// and rechecks it after every await: a result for an account that was signed out, replaced
/// or revoked meanwhile is [`crate::HubError::Stale`], never shown for the new account.
pub trait AccountDevices: Send + Sync {
    /// The devices of the account that are not revoked, the current one included.
    /// [`crate::HubError::NotSignedIn`] when this device holds no credentials for `hub`.
    fn list(&self, hub: &HubId) -> HubFuture<Vec<DeviceSummary>>;

    /// Revokes `device`: its tokens stop working. Revoking this device ends its own session;
    /// the caller signs out instead.
    fn revoke(&self, hub: &HubId, device: &str) -> HubFuture<()>;
}
