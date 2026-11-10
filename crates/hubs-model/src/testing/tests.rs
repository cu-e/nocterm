use futures::executor::block_on;

use super::*;
use crate::Fingerprint;

const ADDRESS: &str = "hub.example:7443";

fn hub_id() -> HubId {
    HubId::from(HubAddress::parse(ADDRESS).unwrap())
}

fn server() -> FakeServer {
    FakeServer::new(ADDRESS, 7)
        .with_user("ada", "secret")
        .with_provider("corp", "Corporate SSO")
}

fn endpoint(trust: TrustPolicy) -> HubEndpointSpec {
    HubEndpointSpec {
        address: HubAddress::parse(ADDRESS).unwrap(),
        trust,
    }
}

fn pinned() -> TrustPolicy {
    TrustPolicy::Pinned(Fingerprint::from_bytes([7; 32]))
}

fn signed_in() -> FakeHub {
    let hub = FakeHub::new().with_server(server());
    block_on(hub.password(endpoint(pinned()), "ada".into(), Secret::new("secret"))).unwrap();
    hub
}

#[test]
fn probe_shows_the_certificate_without_trusting_it() {
    let hub = FakeHub::new().with_server(server());
    let probed = block_on(hub.probe(&HubAddress::parse(ADDRESS).unwrap())).unwrap();
    assert_eq!(probed.fingerprint, Fingerprint::from_bytes([7; 32]));
    assert!(!probed.publicly_trusted);
}

#[test]
fn an_unknown_or_offline_address_is_unreachable() {
    let hub = FakeHub::new().with_server(server());
    let other = HubAddress::parse("nowhere.example").unwrap();
    assert!(matches!(
        block_on(hub.probe(&other)),
        Err(HubError::Unreachable { .. })
    ));
    hub.edit_server(&HubAddress::parse(ADDRESS).unwrap(), |s| s.online = false);
    assert!(matches!(
        block_on(hub.connect(endpoint(pinned()))),
        Err(HubError::Unreachable { .. })
    ));
}

#[test]
fn connect_reports_the_certificate_that_changed_since_it_was_pinned() {
    let hub = FakeHub::new().with_server(server());
    let old = TrustPolicy::Pinned(Fingerprint::from_bytes([1; 32]));
    match block_on(hub.connect(endpoint(old))) {
        Err(HubError::CertificateChanged {
            pinned, presented, ..
        }) => {
            assert_eq!(pinned, Fingerprint::from_bytes([1; 32]));
            assert_eq!(presented, Fingerprint::from_bytes([7; 32]));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn public_trust_needs_a_certificate_a_public_authority_vouches_for() {
    let hub = FakeHub::new().with_server(server());
    assert!(matches!(
        block_on(hub.connect(endpoint(TrustPolicy::PublicCa))),
        Err(HubError::CertificateUntrusted { .. })
    ));
    hub.edit_server(&HubAddress::parse(ADDRESS).unwrap(), |s| {
        s.publicly_trusted = true;
    });
    assert!(block_on(hub.connect(endpoint(TrustPolicy::PublicCa))).is_ok());
}

#[test]
fn connect_tells_whether_this_device_is_signed_in() {
    let hub = FakeHub::new().with_server(server());
    let info = block_on(hub.connect(endpoint(pinned()))).unwrap();
    assert!(!info.signed_in);
    assert_eq!(info.oidc_providers.len(), 1);
    hub.sign_in_as(&hub_id(), "ada");
    assert!(block_on(hub.connect(endpoint(pinned()))).unwrap().signed_in);
}

#[test]
fn connect_refuses_a_hub_that_needs_a_newer_client() {
    let hub = FakeHub::new().with_server(server());
    hub.edit_server(&HubAddress::parse(ADDRESS).unwrap(), |s| s.protocol = 9);
    assert!(matches!(
        block_on(hub.connect(endpoint(pinned()))),
        Err(HubError::UpgradeRequired { hub: 9, .. })
    ));
}

#[test]
fn password_sign_in_returns_the_account_and_remembers_the_device() {
    let hub = FakeHub::new().with_server(server());
    let wrong = block_on(hub.password(endpoint(pinned()), "ada".into(), Secret::new("nope")));
    assert!(matches!(wrong, Err(HubError::InvalidCredentials)));
    assert!(!hub.is_signed_in(&hub_id()));

    let signed_in =
        block_on(hub.password(endpoint(pinned()), "ada".into(), Secret::new("secret"))).unwrap();
    assert_eq!(signed_in.account.username, "ada");
    assert!(!signed_in.account.admin);
    assert_eq!(signed_in.keys, KeyStatus::Open);
    assert!(hub.is_signed_in(&hub_id()));
}

#[test]
fn password_sign_in_is_refused_when_the_hub_offers_only_single_sign_on() {
    let hub = FakeHub::new().with_server(server());
    hub.edit_server(&HubAddress::parse(ADDRESS).unwrap(), |s| {
        s.password_login = false;
    });
    let result = block_on(hub.password(endpoint(pinned()), "ada".into(), Secret::new("secret")));
    assert!(matches!(result, Err(HubError::MethodUnavailable)));
}

#[test]
fn calls_never_record_the_password() {
    let hub = FakeHub::new().with_server(server());
    block_on(hub.password(endpoint(pinned()), "ada".into(), Secret::new("secret"))).unwrap();
    let text = format!("{:?}", hub.calls());
    assert!(text.contains("ada"));
    assert!(!text.contains("secret"));
}

#[test]
fn browser_sign_in_hands_out_the_address_and_finishes_when_the_browser_does() {
    let hub = FakeHub::new().with_server(server());
    let OidcSignIn { browser, done } = hub.oidc(endpoint(pinned()), "corp".into());
    let browser_side = async {
        let url = browser.await.unwrap();
        assert!(url.contains("provider=corp"));
        hub.complete_oidc("ada");
    };
    let (result, ()) = block_on(futures::future::join(done, browser_side));
    assert_eq!(result.unwrap().account.username, "ada");
    assert!(hub.is_signed_in(&hub_id()));
    assert_eq!(hub.cancelled_oidc(), 0);
}

#[test]
fn dropping_a_browser_sign_in_counts_as_cancelling_it() {
    let hub = FakeHub::new().with_server(server());
    let OidcSignIn { browser, mut done } = hub.oidc(endpoint(pinned()), "corp".into());
    block_on(async {
        assert!(futures::poll!(&mut done).is_pending());
        browser.await.unwrap();
    });
    drop(done);
    assert_eq!(hub.cancelled_oidc(), 1);
    assert!(!hub.is_signed_in(&hub_id()));
}

#[test]
fn a_failed_browser_sign_in_reports_why() {
    let hub = FakeHub::new().with_server(server());
    let OidcSignIn { browser, done } = hub.oidc(endpoint(pinned()), "corp".into());
    let browser_side = async {
        browser.await.unwrap();
        hub.fail_oidc(HubError::SignInTimedOut);
    };
    let (result, ()) = block_on(futures::future::join(done, browser_side));
    assert!(matches!(result, Err(HubError::SignInTimedOut)));
    assert!(!hub.is_signed_in(&hub_id()));
}

#[test]
fn an_unknown_provider_is_unavailable() {
    let hub = FakeHub::new().with_server(server());
    let sign_in = hub.oidc(endpoint(pinned()), "other".into());
    assert!(matches!(
        block_on(sign_in.done),
        Err(HubError::MethodUnavailable)
    ));
}

#[test]
fn a_revoked_device_loses_its_sign_in() {
    let hub = signed_in();
    hub.revoke_device(&hub_id());
    assert!(matches!(
        block_on(hub.connect(endpoint(pinned()))),
        Err(HubError::Revoked)
    ));
    assert!(!hub.is_signed_in(&hub_id()));
    assert!(!block_on(hub.connect(endpoint(pinned()))).unwrap().signed_in);
}

#[test]
fn sign_out_and_forget_remove_the_credentials() {
    let hub = signed_in();
    block_on(hub.sign_out(&hub_id())).unwrap();
    assert!(!hub.is_signed_in(&hub_id()));
    hub.sign_in_as(&hub_id(), "ada");
    hub.forget(&hub_id()).unwrap();
    assert!(!hub.is_signed_in(&hub_id()));
    assert!(hub.calls().contains(&FakeCall::Forget(hub_id())));
}

#[test]
fn an_injected_failure_hits_one_call_of_that_kind_only() {
    let hub = signed_in();
    hub.fail_next(FakeOp::Connect, HubError::PermissionDenied);
    assert!(matches!(
        block_on(hub.connect(endpoint(pinned()))),
        Err(HubError::PermissionDenied)
    ));
    assert!(block_on(hub.connect(endpoint(pinned()))).is_ok());
    assert_eq!(hub.count(FakeOp::Connect), 2);
}

#[test]
fn a_held_call_waits_until_released() {
    let hub = signed_in();
    let release = hub.hold(FakeOp::Connect);
    let mut pending = hub.connect(endpoint(pinned()));
    block_on(async { assert!(futures::poll!(&mut pending).is_pending()) });
    release.release();
    assert!(block_on(pending).is_ok());
}

fn keyed_hub() -> FakeHub {
    FakeHub::new().with_server(server().keyed())
}

fn sign_in(hub: &FakeHub, password: &str) -> SignedIn {
    block_on(hub.password(endpoint(pinned()), "ada".into(), Secret::new(password))).unwrap()
}

#[test]
fn a_first_sign_in_creates_the_keys_and_hands_out_the_recovery_key_once() {
    let hub = keyed_hub();
    let KeyStatus::Created(first) = sign_in(&hub, "secret").keys else {
        panic!("created");
    };
    assert_eq!(first.key(), &Secret::new("recovery-ada"));
    assert!(hub.recovery_key_unconfirmed(&hub_id()));
    assert!(matches!(
        sign_in(&hub, "secret").keys,
        KeyStatus::RecoveryKeyUnconfirmed(_)
    ));
    assert!(matches!(block_on(first.confirm()), Err(HubError::Stale)));
    let current = block_on(hub.reissue_recovery_key(&hub_id())).unwrap();
    block_on(current.confirm()).unwrap();
    assert!(!hub.recovery_key_unconfirmed(&hub_id()));
    assert_eq!(sign_in(&hub, "secret").keys, KeyStatus::Open);
}

#[test]
fn reissuing_replaces_the_recovery_key_until_it_is_confirmed() {
    let hub = keyed_hub();
    let KeyStatus::Created(first) = sign_in(&hub, "secret").keys else {
        panic!("created");
    };
    block_on(first.confirm()).unwrap();

    let key = block_on(hub.reissue_recovery_key(&hub_id())).unwrap();
    assert_eq!(key.key(), &Secret::new("recovery-ada+"));
    assert!(hub.recovery_key_unconfirmed(&hub_id()));
    block_on(key.confirm()).unwrap();

    let again = block_on(hub.reissue_recovery_key(&hub_id())).unwrap();
    assert_ne!(again, key);
}

#[test]
fn reissuing_needs_open_keys() {
    let hub = FakeHub::new().with_server(server().with_keyset("ada", "old", "recovery-key"));
    assert_eq!(sign_in(&hub, "secret").keys, KeyStatus::RecoveryNeeded);
    let result = block_on(hub.reissue_recovery_key(&hub_id()));
    assert!(matches!(result, Err(HubError::KeysLocked)));
    assert!(!hub.recovery_key_unconfirmed(&hub_id()));
}

#[test]
fn a_password_that_does_not_open_the_keys_waits_for_the_recovery_key() {
    let hub = FakeHub::new().with_server(server().with_keyset("ada", "old", "recovery-key"));
    assert_eq!(sign_in(&hub, "secret").keys, KeyStatus::RecoveryNeeded);

    let wrong = block_on(hub.recover(&hub_id(), Secret::new("not it")));
    assert!(matches!(wrong, Err(HubError::RecoveryKeyInvalid)));
    block_on(hub.recover(&hub_id(), Secret::new("recovery-key"))).unwrap();
    assert_eq!(sign_in(&hub, "secret").keys, KeyStatus::Open);
}

#[test]
fn recovering_without_a_waiting_sign_in_is_refused() {
    let hub = FakeHub::new().with_server(server().with_keyset("ada", "secret", "recovery-key"));
    sign_in(&hub, "secret");
    let result = block_on(hub.recover(&hub_id(), Secret::new("recovery-key")));
    assert!(matches!(result, Err(HubError::NoRecoveryPending)));
}

#[test]
fn a_browser_sign_in_needs_the_vault_passphrase_to_open_or_make_the_keys() {
    let hub = keyed_hub();
    let OidcSignIn { browser, done } = hub.oidc(endpoint(pinned()), "corp".into());
    let browser_side = async {
        browser.await.unwrap();
        hub.complete_oidc("ada");
    };
    let (result, ()) = block_on(futures::future::join(done, browser_side));
    assert_eq!(result.unwrap().keys, KeyStatus::PassphraseNeeded);

    let made = block_on(hub.set_vault_passphrase(&hub_id(), Secret::new("a long passphrase")));
    let KeyStatus::Created(key) = made.unwrap() else {
        panic!("created");
    };
    assert_eq!(key.key(), &Secret::new("recovery-ada"));
    block_on(key.confirm()).unwrap();
    let again = block_on(hub.set_vault_passphrase(&hub_id(), Secret::new("a long passphrase")));
    assert_eq!(again.unwrap(), KeyStatus::Open);
    let other = block_on(hub.set_vault_passphrase(&hub_id(), Secret::new("another passphrase")));
    assert_eq!(other.unwrap(), KeyStatus::RecoveryNeeded);
}

#[test]
fn an_old_recovery_acknowledgement_cannot_confirm_a_new_reissue() {
    let hub = keyed_hub();
    let KeyStatus::Created(first) = sign_in(&hub, "secret").keys else {
        panic!("created");
    };
    let second = block_on(hub.reissue_recovery_key(&hub_id())).unwrap();
    assert!(matches!(block_on(first.confirm()), Err(HubError::Stale)));
    assert!(hub.recovery_key_unconfirmed(&hub_id()));
    block_on(second.confirm()).unwrap();
    assert!(!hub.recovery_key_unconfirmed(&hub_id()));
}

#[test]
fn a_held_recovery_issue_does_not_choose_a_replacement_account() {
    let hub = keyed_hub();
    sign_in(&hub, "secret");
    let action = hub.recovery_key_offer(&hub_id()).unwrap();
    let held = hub.hold(FakeOp::ReissueRecoveryKey);
    let pending = action.issue();
    hub.sign_in_as(&hub_id(), "bob");
    held.release();
    assert!(matches!(block_on(pending), Err(HubError::Stale)));
}

#[test]
fn a_login_receipt_survives_key_lock_and_ordinary_refresh_but_not_a_new_login() {
    let hub = keyed_hub();
    let signed = sign_in(&hub, "secret");
    hub.lock_keys(&hub_id());
    hub.unlock_keys(&hub_id());
    block_on(hub.connect(endpoint(pinned()))).unwrap();
    assert!(signed.login.check_current().is_ok());
    hub.sign_in_as(&hub_id(), "ada");
    assert!(matches!(signed.login.discard(), Err(HubError::Stale)));
    assert!(hub.is_signed_in(&hub_id()));
}
