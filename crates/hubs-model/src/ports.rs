//! The hub access ports. The application's hub adapter implements them; services use them.
//!
//! Every method returns a runtime-agnostic future that can be awaited on any executor.

use futures::channel::oneshot;
use futures::future::BoxFuture;
use nocterm_session::Secret;

use crate::address::{HubAddress, HubId};
use crate::error::HubError;
use crate::records::{HubEndpointSpec, HubInfo, KeyStatus, SignedIn};
use crate::trust::ProbedHub;

/// The outcome of one hub call.
pub type HubFuture<T> = BoxFuture<'static, Result<T, HubError>>;

/// Reads the certificate of an address nobody has decided to trust yet.
pub trait HubTrust: Send + Sync {
    /// Completes a TLS handshake that accepts any certificate and reports its fingerprint and
    /// whether a public authority vouches for it. Sends no credentials and makes no hub call.
    fn probe(&self, address: &HubAddress) -> HubFuture<ProbedHub>;
}

/// A single sign-in through an identity provider, in two parts.
pub struct OidcSignIn {
    /// Resolves with the address the user must open in a browser, once the sign-in listens.
    pub browser: oneshot::Receiver<String>,
    /// Resolves when the browser sign-in finishes. Dropping it cancels the sign-in and closes
    /// its local listener.
    pub done: HubFuture<SignedIn>,
}

/// Connecting to a hub and signing in or out, with the trust the caller decided.
pub trait HubSignIn: Send + Sync {
    /// Connects with the decided trust and reads what the hub says of itself. The returned
    /// `signed_in` says whether this device already holds working credentials; expired tokens
    /// are renewed on the way. A certificate that differs from the pinned one is
    /// [`HubError::CertificateChanged`]; a revoked device erases its credentials and is
    /// [`HubError::Revoked`].
    fn connect(&self, endpoint: HubEndpointSpec) -> HubFuture<HubInfo>;

    /// Signs in with a user name and password and keeps the credentials on this device. The
    /// password also opens the account's keys, or creates them on a first sign-in: the answer
    /// says which ([`SignedIn::keys`]).
    fn password(
        &self,
        endpoint: HubEndpointSpec,
        username: String,
        password: Secret,
    ) -> HubFuture<SignedIn>;

    /// Signs in through the identity provider `provider` and keeps the credentials. The keys
    /// stay closed until [`HubSignIn::set_vault_passphrase`] opens them.
    fn oidc(&self, endpoint: HubEndpointSpec, provider: String) -> OidcSignIn;

    /// Opens (or, on the first time, creates) the account's keys with a vault passphrase, for an
    /// account that signed in without a password. A passphrase that does not open the keys
    /// leaves them waiting for the recovery key ([`KeyStatus::RecoveryNeeded`]).
    fn set_vault_passphrase(&self, hub: &HubId, passphrase: Secret) -> HubFuture<KeyStatus>;

    /// Opens the keys that wait for the recovery key, and makes the secret typed at sign-in
    /// open them from now on. [`HubError::RecoveryKeyInvalid`] when the key is not the one.
    fn recover(&self, hub: &HubId, recovery_key: Secret) -> HubFuture<()>;

    /// Whether the recovery key made when the account's keys were created on this device was
    /// never confirmed as saved, and the keys are open: a new one can be issued.
    fn recovery_key_unconfirmed(&self, hub: &HubId) -> bool;

    /// How many times the account keys of any hub were opened, dropped or replaced so far. Start
    /// from it to follow [`HubSignIn::keys_changed`].
    fn keys_revision(&self) -> u64;

    /// Resolves with the new count once it differs from `seen`: the keys of some hub were
    /// opened (a vault unlock, a sign-in), dropped or replaced, so what depends on them, such as
    /// [`HubSignIn::recovery_key_unconfirmed`], should be read again.
    fn keys_changed(&self, seen: u64) -> BoxFuture<'static, u64>;

    /// Issues a new recovery key with the open keys and replaces the hub's copy of the old one;
    /// the new key is returned once and kept nowhere, but marked unconfirmed until
    /// [`HubSignIn::confirm_recovery_key`]. [`HubError::KeysLocked`] while the keys are closed.
    fn reissue_recovery_key(&self, hub: &HubId) -> HubFuture<Secret>;

    /// Records that the person saved the recovery key shown last.
    fn confirm_recovery_key(&self, hub: &HubId) -> Result<(), HubError>;

    /// Ends this device's session on the hub and erases its credentials, even when the hub
    /// cannot be reached.
    fn sign_out(&self, hub: &HubId) -> HubFuture<()>;

    /// Erases this device's credentials for the hub without contacting it.
    fn forget(&self, hub: &HubId) -> Result<(), HubError>;
}
