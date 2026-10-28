use std::path::PathBuf;
use std::sync::Arc;

use nocterm_hub_client::{ClientError, RelayError as HubRelayError};
use nocterm_hubs_model::{HubAddress, HubError};
use tonic::{Code, Status};

use super::*;

fn pool() -> Pool {
    Pool::new(
        PathBuf::new(),
        "test",
        Arc::new(crate::account::Accounts::new(PathBuf::new())),
    )
}

fn hub() -> HubId {
    HubId::from(HubAddress::parse("hub.example").unwrap())
}

fn open(node: Option<&str>, error: HubRelayError) -> RelayError {
    opening(&pool(), &hub(), "hub.example", node, error)
}

fn closed(code: ErrorCode) -> HubRelayError {
    HubRelayError::Closed {
        code: Some(code),
        message: "text that is never read".into(),
    }
}

#[test]
fn the_hubs_refusals_are_the_sessions_refusals() {
    assert_eq!(
        open(None, closed(ErrorCode::RelayDenied)),
        RelayError::Refused {
            reason: RelayRefusal::Policy
        }
    );
    assert_eq!(
        open(None, closed(ErrorCode::RelayLimit)),
        RelayError::Refused {
            reason: RelayRefusal::Limit
        }
    );
}

#[test]
fn a_refusal_in_a_status_is_a_refusal_too() {
    let mut status = Status::new(Code::PermissionDenied, "no");
    status.metadata_mut().insert(
        "x-nocterm-error",
        "ERROR_CODE_RELAY_DENIED".parse().unwrap(),
    );
    assert_eq!(
        open(None, HubRelayError::from(status)),
        RelayError::Refused {
            reason: RelayRefusal::Policy
        }
    );
}

#[test]
fn an_unavailable_node_names_the_node_that_was_asked_for() {
    assert_eq!(
        open(Some("build-1"), closed(ErrorCode::NodeUnavailable)),
        RelayError::NodeUnavailable {
            node: "build-1".into()
        }
    );
    // Without a node the hub itself dialled, so no node is to blame.
    assert!(matches!(
        open(None, closed(ErrorCode::NodeUnavailable)),
        RelayError::Unreachable(_)
    ));
}

#[test]
fn a_hub_without_relays_is_unreachable_and_keeps_its_cause() {
    let RelayError::Unreachable(cause) = open(None, HubRelayError::Unsupported) else {
        panic!("expected Unreachable");
    };
    assert!(matches!(
        cause.downcast_ref::<HubRelayError>(),
        Some(HubRelayError::Unsupported)
    ));
}

#[test]
fn a_broken_stream_is_unreachable() {
    for error in [
        HubRelayError::Interrupted,
        HubRelayError::UnexpectedFrame,
        HubRelayError::BadPeerAddress,
        closed(ErrorCode::Unspecified),
    ] {
        assert!(matches!(open(None, error), RelayError::Unreachable(_)));
    }
}

#[test]
fn tokens_the_hub_no_longer_accepts_mean_not_signed_in() {
    let rejected = HubRelayError::from(Status::new(Code::Unauthenticated, "expired"));
    assert_eq!(
        open(None, rejected),
        RelayError::NotSignedIn {
            hub: "hub.example".into()
        }
    );
    assert_eq!(
        open(None, HubRelayError::Hub(ClientError::NotSignedIn)),
        RelayError::NotSignedIn {
            hub: "hub.example".into()
        }
    );
}

#[test]
fn a_hub_that_does_not_answer_is_unreachable_and_keeps_its_cause() {
    let down = HubRelayError::from(Status::new(Code::Unavailable, "down"));
    let RelayError::Unreachable(cause) = open(None, down) else {
        panic!("expected Unreachable");
    };
    assert!(matches!(
        cause.downcast_ref::<HubError>(),
        Some(HubError::Unreachable { .. })
    ));
}

#[test]
fn a_hub_without_a_client_is_not_signed_in_unless_it_failed_otherwise() {
    assert_eq!(
        without_client("h", HubError::NotSignedIn),
        RelayError::NotSignedIn { hub: "h".into() }
    );
    assert_eq!(
        without_client("h", HubError::Revoked),
        RelayError::NotSignedIn { hub: "h".into() }
    );
    assert!(matches!(
        without_client("h", HubError::NotFound),
        RelayError::Unreachable(_)
    ));
}
