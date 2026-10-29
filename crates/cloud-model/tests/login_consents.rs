mod support;
use nocterm_cloud_model::{
    authority::{AuthenticationBinding, TargetBinding},
    identity::{AttemptId, Fingerprint, NamespaceId, Timestamp},
    login::*,
};
use nocterm_ssh_keys::testing::LoginProofFixture;
fn now(value: u64) -> Timestamp {
    Timestamp::from_millis(value)
}
fn binding(f: &LoginProofFixture, id: u8) -> LoginConsentBinding {
    LoginConsentBinding::new(
        support::run(1, 2, 3, 1),
        TargetBinding::new(
            Fingerprint::from_bytes(f.host_key().fingerprint_bytes().unwrap()),
            support::fingerprint(5),
            NamespaceId::new("alice").unwrap(),
            support::fingerprint(6),
        ),
        AuthenticationBinding::new(
            Fingerprint::from_bytes(f.user_key().fingerprint_bytes().unwrap()),
            support::fingerprint(7),
        ),
        AttemptId::from_bytes([id; 16]),
        [8; 32],
        LoginConsentWindow::new(now(1), now(60001)).unwrap(),
        f.user_key(),
    )
    .unwrap()
}
fn request(f: &LoginProofFixture, b: LoginConsentBinding) -> LoginProofRequest {
    LoginProofRequest::new(b, f.host_proof([9; 32]), f.login("alice", [9; 32]))
}
#[test]
fn consent_is_single_use_and_a_fresh_ledger_cannot_restore_it_from_request() {
    let f = LoginProofFixture::default();
    let b = binding(&f, 1);
    let r = request(&f, b.clone());
    let mut ledger = LoginConsentLedger::new(now(1));
    assert!(matches!(
        ledger.consume(&r, now(1)),
        Err(LoginError::NeedsUnlock)
    ));
    ledger.issue(b, f.host_key(), now(1)).unwrap();
    ledger.consume(&r, now(1)).unwrap();
    assert!(matches!(
        ledger.consume(&r, now(1)),
        Err(LoginError::Replay)
    ));
    assert!(matches!(
        LoginConsentLedger::new(now(1)).consume(&r, now(1)),
        Err(LoginError::NeedsUnlock)
    ));
}
#[test]
fn changed_nonce_principal_session_host_or_route_never_consumes_original_consent() {
    let f = LoginProofFixture::default();
    let b = binding(&f, 1);
    let mut ledger = LoginConsentLedger::new(now(1));
    ledger.issue(b.clone(), f.host_key(), now(1)).unwrap();
    let cases = [
        LoginProofRequest::new(b.clone(), f.host_proof([0; 32]), f.login("alice", [9; 32])),
        LoginProofRequest::new(b.clone(), f.host_proof([9; 32]), f.login("bob", [9; 32])),
        request(&f, binding(&f, 2)),
    ];
    for r in cases {
        assert!(ledger.consume(&r, now(1)).is_err());
    }
    ledger.consume(&request(&f, b), now(1)).unwrap();
}
#[test]
fn finite_expiry_and_clock_rollback_require_fresh_unlock() {
    let f = LoginProofFixture::default();
    let b = binding(&f, 1);
    let r = request(&f, b.clone());
    let mut ledger = LoginConsentLedger::new(now(1));
    ledger.issue(b, f.host_key(), now(1)).unwrap();
    assert!(matches!(
        ledger.consume(&r, now(60001)),
        Err(LoginError::Expired)
    ));
    assert!(matches!(
        ledger.consume(&r, now(2)),
        Err(LoginError::ClockRollback)
    ));
    assert!(matches!(
        ledger.consume(&r, now(60002)),
        Err(LoginError::NeedsUnlock)
    ));
    assert!(LoginConsentWindow::new(now(1), now(60002)).is_err());
}
#[test]
fn full_live_ledger_does_not_evict_another_devices_original_attempt() {
    let f = LoginProofFixture::default();
    let mut ledger = LoginConsentLedger::new(now(1));
    for id in 1..=MAX_LOGIN_CONSENTS as u8 {
        ledger.issue(binding(&f, id), f.host_key(), now(1)).unwrap();
    }
    assert!(matches!(
        ledger.issue(binding(&f, 100), f.host_key(), now(1)),
        Err(LoginError::Capacity)
    ));
    ledger
        .consume(&request(&f, binding(&f, 1)), now(1))
        .unwrap();
}
