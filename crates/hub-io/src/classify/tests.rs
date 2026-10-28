use nocterm_hub_client::{ConnectError, EndpointError, OidcError};
use tonic::{Code, Status};

use super::*;

fn address() -> HubAddress {
    HubAddress::parse("hub.example").unwrap()
}

fn status(code: Code) -> ClientError {
    ClientError::from(Status::new(code, "details the user never sees"))
}

fn coded(code: Code, hub_code: &str) -> ClientError {
    let mut status = Status::new(code, "revoked");
    status
        .metadata_mut()
        .insert("x-nocterm-error", hub_code.parse().unwrap());
    ClientError::from(status)
}

#[test]
fn a_revoked_device_is_revoked_however_the_hub_says_it() {
    assert!(matches!(
        classify(&address(), ClientError::Revoked),
        HubError::Revoked
    ));
    let wire = coded(Code::Unauthenticated, "ERROR_CODE_DEVICE_REVOKED");
    assert!(matches!(classify(&address(), wire), HubError::Revoked));
}

#[test]
fn statuses_become_the_errors_a_caller_acts_on() {
    assert!(matches!(
        classify(&address(), status(Code::Unauthenticated)),
        HubError::NotSignedIn
    ));
    assert!(matches!(
        classify(&address(), status(Code::PermissionDenied)),
        HubError::PermissionDenied
    ));
    assert!(matches!(
        classify(&address(), status(Code::NotFound)),
        HubError::NotFound
    ));
    for code in [Code::Unavailable, Code::DeadlineExceeded] {
        assert!(matches!(
            classify(&address(), status(code)),
            HubError::Unreachable { .. }
        ));
    }
    assert!(matches!(
        classify(&address(), status(Code::Internal)),
        HubError::Remote { .. }
    ));
}

#[test]
fn an_old_client_is_told_both_versions() {
    let error = ClientError::UpgradeRequired { hub: 3, client: 1 };
    assert!(matches!(
        classify(&address(), error),
        HubError::UpgradeRequired { hub: 3, client: 1 }
    ));
}

#[test]
fn a_connection_that_failed_is_unreachable_and_keeps_its_cause() {
    let error = ClientError::Connect(ConnectError::Endpoint(EndpointError::Address));
    let classified = classify(&address(), error);
    let HubError::Unreachable { address: at, .. } = &classified else {
        panic!("{classified:?}");
    };
    assert_eq!(at.to_string(), "hub.example:7443");
    let cause = classified.source().unwrap();
    let inner = cause
        .source()
        .and_then(|source| source.downcast_ref::<ClientError>())
        .or_else(|| cause.downcast_ref::<ClientError>());
    assert!(matches!(inner, Some(ClientError::Connect(_))), "{cause:?}");
}

#[test]
fn a_failure_to_keep_tokens_is_a_credentials_error() {
    let error = ClientError::SessionStore(std::io::Error::other("disk"));
    assert!(matches!(
        classify(&address(), error),
        HubError::Credentials { .. }
    ));
}

#[test]
fn browser_sign_in_failures_say_why() {
    let timed_out = ClientError::Oidc(OidcError::TimedOut);
    assert!(matches!(
        classify_sign_in(&address(), Method::Oidc, timed_out),
        HubError::SignInTimedOut
    ));
    let refused = ClientError::Oidc(OidcError::Provider {
        error: "access_denied".into(),
        description: String::new(),
    });
    assert!(matches!(
        classify_sign_in(&address(), Method::Oidc, refused),
        HubError::SignInCancelled
    ));
    let loopback = ClientError::Oidc(OidcError::Loopback(std::io::Error::other("bind")));
    assert!(matches!(
        classify_sign_in(&address(), Method::Oidc, loopback),
        HubError::Remote { .. }
    ));
}

#[test]
fn a_refused_password_is_invalid_credentials_only_when_signing_in() {
    for code in [
        Code::Unauthenticated,
        Code::InvalidArgument,
        Code::PermissionDenied,
    ] {
        assert!(matches!(
            classify_sign_in(&address(), Method::Password, status(code)),
            HubError::InvalidCredentials
        ));
    }
    assert!(matches!(
        classify(&address(), status(Code::InvalidArgument)),
        HubError::Remote { .. }
    ));
}

#[test]
fn a_password_too_short_for_any_account_is_invalid_credentials() {
    let short = || ClientError::Login(LoginError::PasswordTooShort { min: 8 });
    assert!(matches!(
        classify_sign_in(&address(), Method::Password, short()),
        HubError::InvalidCredentials
    ));
}

#[test]
fn a_method_the_hub_does_not_offer_is_unavailable() {
    for method in [Method::Password, Method::Oidc] {
        assert!(matches!(
            classify_sign_in(&address(), method, status(Code::Unimplemented)),
            HubError::MethodUnavailable
        ));
    }
    assert!(matches!(
        classify_sign_in(&address(), Method::Oidc, status(Code::NotFound)),
        HubError::MethodUnavailable
    ));
}

#[test]
fn a_hub_that_is_not_there_is_unreachable_not_a_changed_certificate() {
    let spec = HubEndpointSpec {
        address: HubAddress::parse("127.0.0.1:1").unwrap(),
        trust: TrustPolicy::Pinned(Fingerprint::from_bytes([1; 32])),
    };
    let error = ClientError::Connect(ConnectError::Endpoint(EndpointError::Address));
    let classified = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(explain_connect(&spec, error));
    assert!(
        matches!(classified, HubError::Unreachable { .. }),
        "{classified:?}"
    );
}
