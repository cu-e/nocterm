//! Typed failures of hub addresses, fingerprints and hub access.

use nocterm_core::error::SharedCause;

use crate::address::HubAddress;
use crate::trust::Fingerprint;

/// Why a typed hub address was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HubAddressError {
    #[error("the hub address is empty")]
    Empty,
    #[error("the hub address has no valid host name")]
    InvalidHost,
    #[error("the hub address has no valid port")]
    InvalidPort,
    /// `http://` has no certificate to check, so a hub is never reached over it.
    #[error("plain http:// hubs are refused; hubs use TLS")]
    PlaintextRefused,
    #[error("the hub address has an unsupported scheme; use https:// or no scheme")]
    UnsupportedScheme,
}

/// Why a typed certificate fingerprint was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FingerprintError {
    /// A SHA-256 fingerprint has 64 hexadecimal digits; `found` digits were typed.
    #[error("a fingerprint has 64 hexadecimal digits, found {found}")]
    Length { found: usize },
    #[error("a fingerprint holds only hexadecimal digits")]
    NotHex,
}

/// What went wrong reaching or using a hub, in terms the caller can act on.
///
/// The original failure stays reachable through `source()`; no variant carries text.
#[derive(Clone, Debug, thiserror::Error)]
pub enum HubError {
    /// The hub did not answer.
    #[error("cannot reach the hub at {address}")]
    Unreachable {
        address: HubAddress,
        #[source]
        cause: SharedCause,
    },
    /// The hub presents another certificate than the pinned one. Never retried and never
    /// accepted without the user removing the hub and signing in again.
    #[error("the certificate of {address} changed")]
    CertificateChanged {
        address: HubAddress,
        pinned: Fingerprint,
        presented: Fingerprint,
    },
    /// The certificate is not vouched for by a public authority and nothing is pinned.
    #[error("the certificate of {address} is not trusted")]
    CertificateUntrusted {
        address: HubAddress,
        presented: Fingerprint,
    },
    /// The hub and this client speak different protocol versions.
    #[error("the hub speaks protocol {hub}, this client speaks {client}")]
    UpgradeRequired { hub: u32, client: u32 },
    /// There are no usable credentials for this hub on this device.
    #[error("not signed in to the hub")]
    NotSignedIn,
    /// The hub revoked this device; its credentials are gone.
    #[error("this device was revoked by the hub")]
    Revoked,
    #[error("the user name or password was refused")]
    InvalidCredentials,
    /// The hub does not offer the requested sign-in method.
    #[error("the hub does not offer this way of signing in")]
    MethodUnavailable,
    #[error("signing in took too long")]
    SignInTimedOut,
    #[error("signing in was cancelled")]
    SignInCancelled,
    #[error("the hub refused the request for lack of permission")]
    PermissionDenied,
    #[error("the hub has no such object")]
    NotFound,
    /// The credential file of this device could not be read or written.
    #[error("the credentials of this device cannot be kept")]
    Credentials {
        #[source]
        cause: SharedCause,
    },
    /// Any other failure of the hub client.
    #[error("the hub request failed")]
    Remote {
        #[source]
        cause: SharedCause,
    },
}
