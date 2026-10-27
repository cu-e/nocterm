//! The hub access ports. The application's hub adapter implements them; services use them.
//!
//! Every method returns a runtime-agnostic future that can be awaited on any executor.

use std::sync::Arc;

use futures::channel::oneshot;
use futures::future::BoxFuture;
use nocterm_session::{Secret, Transport};

use crate::address::{HubAddress, HubId};
use crate::error::HubError;
use crate::records::{
    Account, HubEndpointSpec, HubInfo, HubSessionId, HubSessionSummary, HubTerminalTarget,
    NodeSummary, OrgId,
};
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
    pub done: HubFuture<Account>,
}

/// Connecting to a hub and signing in or out, with the trust the caller decided.
pub trait HubSignIn: Send + Sync {
    /// Connects with the decided trust and reads what the hub says of itself. The returned
    /// `signed_in` says whether this device already holds working credentials; expired tokens
    /// are renewed on the way. A certificate that differs from the pinned one is
    /// [`HubError::CertificateChanged`]; a revoked device erases its credentials and is
    /// [`HubError::Revoked`].
    fn connect(&self, endpoint: HubEndpointSpec) -> HubFuture<HubInfo>;

    /// Signs in with a user name and password and keeps the credentials on this device.
    fn password(
        &self,
        endpoint: HubEndpointSpec,
        username: String,
        password: Secret,
    ) -> HubFuture<Account>;

    /// Signs in through the identity provider `provider` and keeps the credentials.
    fn oidc(&self, endpoint: HubEndpointSpec, provider: String) -> OidcSignIn;

    /// Ends this device's session on the hub and erases its credentials, even when the hub
    /// cannot be reached.
    fn sign_out(&self, hub: &HubId) -> HubFuture<()>;

    /// Erases this device's credentials for the hub without contacting it.
    fn forget(&self, hub: &HubId) -> Result<(), HubError>;
}

/// What the window of one hub reads and does on the hub.
pub trait HubSessions: Send + Sync {
    /// The account this device is signed in as, with its organizations.
    fn account(&self, hub: &HubId) -> HubFuture<Account>;

    fn nodes(&self, hub: &HubId, org: &OrgId) -> HubFuture<Vec<NodeSummary>>;

    /// Sessions that have not ended.
    fn sessions(&self, hub: &HubId, org: &OrgId) -> HubFuture<Vec<HubSessionSummary>>;

    /// Terminates the process of a session on its node.
    fn end(&self, hub: &HubId, session: &HubSessionId) -> HubFuture<()>;

    /// A transport attached to one hub session, for a terminal tab.
    ///
    /// The transport is bound to its target: it ignores the destination fields of the
    /// `ConnectRequest` and uses only its size. Opening it attaches (creating the session first
    /// for [`HubTerminalTarget::New`]); the screen of an existing session arrives as
    /// `Event::Restore`. Closing the session detaches and leaves the process running on its
    /// node; [`HubSessions::end`] terminates it. Failures arrive as `Event::Closed`.
    fn terminal(&self, hub: &HubId, target: HubTerminalTarget) -> Arc<dyn Transport>;
}
