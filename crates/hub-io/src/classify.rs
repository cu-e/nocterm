//! What went wrong, in the terms of `HubError`.
//!
//! Every function keeps the original failure as the `source()` of the error it returns.

use std::error::Error;

use nocterm_core::SharedCause;
use nocterm_hub_client::{ClientError, HubEndpoint, OidcError, ProbeError, StatusCode};
use nocterm_hubs_model::{Fingerprint, HubAddress, HubEndpointSpec, HubError, TrustPolicy};
use nocterm_keys::LoginError;

use crate::credentials::CredentialError;

pub(crate) fn remote(error: impl Error + Send + Sync + 'static) -> HubError {
    HubError::Remote {
        cause: SharedCause::new(error),
    }
}

pub(crate) fn credentials(error: impl Error + Send + Sync + 'static) -> HubError {
    HubError::Credentials {
        cause: SharedCause::new(error),
    }
}

pub(crate) fn credential_file(error: CredentialError) -> HubError {
    match error {
        CredentialError::StaleLogin => HubError::Stale,
        other => credentials(other),
    }
}

/// Whether the hub cannot be asked now: it does not answer, or this device has no (more)
/// credentials for it. Work that needs the hub waits; work that has a local copy goes on.
pub(crate) fn unreachable(error: &HubError) -> bool {
    matches!(
        error,
        HubError::Unreachable { .. } | HubError::NotSignedIn | HubError::Revoked
    )
}

/// How a call to a hub that was signed in to, or not, is classified.
enum Kind {
    Settled(HubError),
    Unreachable,
    Credentials,
    Remote,
}

fn kind(error: &ClientError) -> Kind {
    match error {
        ClientError::UpgradeRequired { hub, client } => Kind::Settled(HubError::UpgradeRequired {
            hub: *hub,
            client: *client,
        }),
        ClientError::NotSignedIn => Kind::Settled(HubError::NotSignedIn),
        ClientError::Revoked => Kind::Settled(HubError::Revoked),
        ClientError::Oidc(OidcError::TimedOut) => Kind::Settled(HubError::SignInTimedOut),
        ClientError::Oidc(OidcError::Provider { .. } | OidcError::NoCode) => {
            Kind::Settled(HubError::SignInCancelled)
        }
        ClientError::SessionStore(_) | ClientError::Randomness(_) => Kind::Credentials,
        ClientError::Connect(_) => Kind::Unreachable,
        ClientError::Status { code, .. } => match code {
            StatusCode::Unauthenticated => Kind::Settled(HubError::NotSignedIn),
            StatusCode::PermissionDenied => Kind::Settled(HubError::PermissionDenied),
            StatusCode::NotFound => Kind::Settled(HubError::NotFound),
            StatusCode::Unavailable | StatusCode::DeadlineExceeded | StatusCode::Cancelled => {
                Kind::Unreachable
            }
            _ => Kind::Remote,
        },
        _ => Kind::Remote,
    }
}

/// A failure of a call on a hub at `address`.
pub(crate) fn classify(address: &HubAddress, error: ClientError) -> HubError {
    match kind(&error) {
        Kind::Settled(settled) => settled,
        Kind::Unreachable => HubError::Unreachable {
            address: address.clone(),
            cause: SharedCause::new(error),
        },
        Kind::Credentials => credentials(error),
        Kind::Remote => remote(error),
    }
}

/// Which sign-in method failed.
#[derive(Clone, Copy)]
pub(crate) enum Method {
    Password,
    Oidc,
}

/// A failure of a sign-in: a refusal means something else here than on any other call.
pub(crate) fn classify_sign_in(
    address: &HubAddress,
    method: Method,
    error: ClientError,
) -> HubError {
    // A password the hub could not have accepted is refused before it is sent.
    if let (
        Method::Password,
        ClientError::Login(LoginError::PasswordTooShort { .. } | LoginError::EmptyUsername),
    ) = (method, &error)
    {
        return HubError::InvalidCredentials;
    }
    if let ClientError::Status { code, .. } = &error {
        match (method, code) {
            (
                Method::Password,
                StatusCode::Unauthenticated
                | StatusCode::InvalidArgument
                | StatusCode::NotFound
                | StatusCode::PermissionDenied,
            ) => return HubError::InvalidCredentials,
            (_, StatusCode::FailedPrecondition | StatusCode::Unimplemented)
            | (Method::Oidc, StatusCode::NotFound | StatusCode::InvalidArgument) => {
                return HubError::MethodUnavailable;
            }
            _ => {}
        }
    }
    classify(address, error)
}

/// A failure to connect. The client only says the connection failed, so the certificate the
/// hub presents is read and compared with the decided trust to tell a changed certificate
/// from a hub that is simply not there.
pub(crate) async fn explain_connect(spec: &HubEndpointSpec, error: ClientError) -> HubError {
    if !matches!(error, ClientError::Connect(_)) {
        return classify(&spec.address, error);
    }
    let Ok(probe) = HubEndpoint::probe(&spec.address.to_string()).await else {
        return classify(&spec.address, error);
    };
    let presented = Fingerprint::from_bytes(probe.fingerprint);
    match spec.trust {
        TrustPolicy::Pinned(pinned) if pinned != presented => HubError::CertificateChanged {
            address: spec.address.clone(),
            pinned,
            presented,
        },
        TrustPolicy::PublicCa if !probe.publicly_trusted => HubError::CertificateUntrusted {
            address: spec.address.clone(),
            presented,
        },
        _ => classify(&spec.address, error),
    }
}

/// A failure to read the certificate of a hub.
pub(crate) fn probe_failed(address: &HubAddress, error: ProbeError) -> HubError {
    match error {
        ProbeError::Unreachable(_)
        | ProbeError::Handshake(_)
        | ProbeError::TimedOut
        | ProbeError::NoCertificate => HubError::Unreachable {
            address: address.clone(),
            cause: SharedCause::new(error),
        },
        _ => remote(error),
    }
}

#[cfg(test)]
mod tests;
