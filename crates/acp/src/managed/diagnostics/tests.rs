//! The original typed adapter remains the independent fault projection oracle.
use super::original;
use super::*;
use crate::managed::{tests::fixture, wire};
use agent_client_protocol::managed::{Frame, Source};
#[test]
fn managed_fault_detail_precedence_redaction_unicode_caps_and_categories_match_original() {
    let rules = Arc::new(DiagnosticRedactor::new().unwrap());
    for (code, message, data, restoring) in [
        (
            -32000,
            "authenticate",
            serde_json::json!("api_key=secret-value"),
            true,
        ),
        (
            -32601,
            "missing",
            serde_json::json!({"message":"first","details":"second","error":{"message":"last"}}),
            true,
        ),
        (
            -32001,
            "UNKNOWN SESSION",
            serde_json::json!({"details":"detail","error":"ignored"}),
            true,
        ),
        (
            -32001,
            "normal",
            serde_json::json!({"message":null,"details":false,"error":{"message":"nested Ω"}}),
            true,
        ),
        (
            -32001,
            "normal",
            serde_json::json!({"error":"third"}),
            false,
        ),
        (-32001, "normal", serde_json::json!(["ignored"]), false),
    ] {
        let fixture = fixture();
        let mut secrets = Buffer::new(fixture.budget.clone());
        secrets
            .push(Source::copy("secret-value", fixture.budget.clone()).unwrap())
            .map_err(|(error, _)| error)
            .unwrap();
        let diagnostics = Diagnostics::new(rules.clone(), secrets, fixture.budget.clone());
        let error = agent_client_protocol_schema::v1::Error::new(code, message).data(data);
        let raw = serde_json::to_string(&serde_json::json!({"jsonrpc":"2.0","id":1,"error":error}))
            .unwrap();
        let source = Source::copy(&raw, fixture.budget.clone()).unwrap();
        let Frame::Single(envelope) = wire::frame(&source, fixture.budget.clone()).unwrap() else {
            panic!("fault frame")
        };
        let actual = diagnostics.remote(&envelope, restoring).unwrap();
        let original = original::map_error_with(error, &["secret-value".to_owned()]);
        let expected = if restoring {
            original::restore_error(original)
        } else {
            original
        };
        assert_eq!(actual.to_string(), expected.to_string());
    }
}
#[test]
fn restore_classification_uses_the_original_final4096_scalars_after_detail2048_scrub() {
    let rules = Arc::new(DiagnosticRedactor::new().unwrap());
    for message in [
        format!("{} unknown session", "界".repeat(4096)),
        format!("{} UNKNOWN SESSION", "界".repeat(4080)),
    ] {
        let fixture = fixture();
        let diagnostics = Diagnostics::new(
            rules.clone(),
            Buffer::new(fixture.budget.clone()),
            fixture.budget.clone(),
        );
        let error =
            agent_client_protocol_schema::v1::Error::new(-32001, message).data("Ω".repeat(5000));
        let raw = serde_json::to_string(&serde_json::json!({"jsonrpc":"2.0","id":1,"error":error}))
            .unwrap();
        let source = Source::copy(&raw, fixture.budget.clone()).unwrap();
        let Frame::Single(envelope) = wire::frame(&source, fixture.budget.clone()).unwrap() else {
            panic!("fault frame")
        };
        let actual = diagnostics.remote(&envelope, true).unwrap();
        let expected = original::restore_error(original::map_error_with(error, &[]));
        assert_eq!(actual.to_string(), expected.to_string());
    }
}
fn remote(error: &agent_client_protocol_schema::v1::Error, restoring: bool) -> ManagedCommandError {
    let fixture = fixture();
    let mut secrets = Buffer::new(fixture.budget.clone());
    secrets
        .push(Source::copy("custom-auth-marker", fixture.budget.clone()).unwrap())
        .map_err(|(error, _)| error)
        .unwrap();
    let diagnostics = Diagnostics::new(
        Arc::new(DiagnosticRedactor::new().unwrap()),
        secrets,
        fixture.budget.clone(),
    );
    let raw =
        serde_json::to_string(&serde_json::json!({"jsonrpc":"2.0","id":1,"error":error})).unwrap();
    let source = Source::copy(&raw, fixture.budget.clone()).unwrap();
    let Frame::Single(envelope) = wire::frame(&source, fixture.budget.clone()).unwrap() else {
        panic!("fault frame")
    };
    diagnostics.remote(&envelope, restoring).unwrap()
}
fn kind(error: &ManagedCommandError) -> RemoteFaultKind {
    let ManagedCommandError::Remote(fault) = error else {
        panic!("remote fault")
    };
    fault.get().kind()
}
#[test]
fn remote_error_payloads_expose_only_scrubbed_detail() {
    use agent_client_protocol_schema::v1::Error;
    let error = remote(
        &Error::auth_required().data(serde_json::json!({"password":"marker"})),
        false,
    );
    assert_eq!(kind(&error), RemoteFaultKind::AuthenticationRequired);
    let error = remote(
        &Error::internal_error().data(serde_json::json!({
            "message": "Session limit reached; custom-auth-marker",
            "unrelated": "do not expose",
        })),
        false,
    );
    let message = error.to_string();
    assert!(message.contains("Session limit reached"));
    assert!(!message.contains("custom-auth-marker"));
    assert!(!message.contains("do not expose"));
}
#[test]
fn missing_rollout_is_unavailable_only_for_restore_rpc_faults() {
    use agent_client_protocol_schema::v1::Error;
    let missing = Error::internal_error().data(serde_json::json!({
        "message": "no rollout found for thread id 01a11ca8-4cde-72e3-8429-f13aaec4f58c"
    }));
    assert_eq!(kind(&remote(&missing, false)), RemoteFaultKind::Rpc);
    assert_eq!(
        kind(&remote(&missing, true)),
        RemoteFaultKind::RestoreUnavailable
    );
    for message in [
        "storage temporarily unavailable",
        "rollout write failed",
        "thread id invalid",
        "no rollout found",
        "no rollout found for thread identity",
    ] {
        let error = Error::internal_error().data(message);
        assert_eq!(kind(&remote(&error, true)), RemoteFaultKind::Rpc);
    }
    let error = Error::auth_required().data("no rollout found for thread id old");
    assert_eq!(
        kind(&remote(&error, true)),
        RemoteFaultKind::AuthenticationRequired
    );
}
