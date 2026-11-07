//! A failed relay in the terms of `nocterm-session`'s `RelayError`.
//!
//! The contract names a refusal by a code and keeps its text for display, so the code decides
//! and the text is never read. The original failure stays the `source()` of what is returned.

use nocterm_core::SharedCause;
use nocterm_hub_client::{ClientError, RelayError as HubRelayError};
use nocterm_hub_proto::v2::ErrorCode;
use nocterm_hubs_model::HubError;
use nocterm_session::{RelayError, RelayRefusal};

use crate::pool::captured::CapturedClient;

/// Why the hub named by `name` has no usable client.
pub(super) fn without_client(name: &str, error: HubError) -> RelayError {
    match error {
        HubError::NotSignedIn | HubError::Revoked => RelayError::NotSignedIn {
            hub: name.to_owned(),
        },
        other => cannot_reach(other),
    }
}

/// Why the hub did not open a relay to a destination.
///
/// A device the hub revoked is forgotten on the spot, as for any other call.
pub(super) fn opening(client: &CapturedClient, name: &str, error: HubRelayError) -> RelayError {
    classify_opening(name, error, |error| client.failure(error))
}
fn classify_opening(
    name: &str,
    error: HubRelayError,
    failed: impl FnOnce(ClientError) -> HubError,
) -> RelayError {
    if let Some(code) = code_of(&error)
        && let Some(refused) = coded(code)
    {
        return refused;
    }
    match error {
        HubRelayError::Hub(error) => without_client(name, failed(error)),
        other => cannot_reach(other),
    }
}

/// The hub's own reason for a failure, when it gave one.
fn code_of(error: &HubRelayError) -> Option<ErrorCode> {
    match error {
        HubRelayError::Closed { code, .. }
        | HubRelayError::Hub(ClientError::Status { hub_code: code, .. }) => *code,
        _ => None,
    }
}

/// What a hub's code says about a relay, if it says anything.
fn coded(code: ErrorCode) -> Option<RelayError> {
    match code {
        // The contract does not say which rule refused, only that one did.
        ErrorCode::RelayDenied => Some(RelayError::Refused {
            reason: RelayRefusal::Policy,
        }),
        ErrorCode::RelayLimit => Some(RelayError::Refused {
            reason: RelayRefusal::Limit,
        }),
        _ => None,
    }
}

fn cannot_reach(cause: impl std::error::Error + Send + Sync + 'static) -> RelayError {
    RelayError::Unreachable(SharedCause::new(cause))
}

#[cfg(test)]
mod tests;
