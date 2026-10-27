use futures::executor::block_on;

use super::*;

const ADDRESS: &str = "hub.example:7443";

fn hub_id() -> HubId {
    HubId::from(HubAddress::parse(ADDRESS).unwrap())
}

fn server() -> FakeServer {
    FakeServer::new(ADDRESS, 7)
        .with_user("ada", "secret", "lab")
        .with_provider("corp", "Corporate SSO")
        .with_node("n1", "build box", true)
        .with_session("s1", "lab", "n1", "ada", "make")
        .with_session("s2", "lab", "n1", "bob", "top")
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

    let account =
        block_on(hub.password(endpoint(pinned()), "ada".into(), Secret::new("secret"))).unwrap();
    assert_eq!(account.username, "ada");
    assert_eq!(account.orgs[0].id.as_str(), "lab");
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
    assert_eq!(result.unwrap().username, "ada");
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
fn lists_need_a_signed_in_device() {
    let hub = FakeHub::new().with_server(server());
    let org = OrgId::new("lab");
    assert!(matches!(
        block_on(hub.nodes(&hub_id(), &org)),
        Err(HubError::NotSignedIn)
    ));
    assert!(matches!(
        block_on(hub.account(&hub_id())),
        Err(HubError::NotSignedIn)
    ));
}

#[test]
fn signed_in_device_lists_nodes_and_only_running_sessions_of_the_organization() {
    let hub = signed_in();
    let org = OrgId::new("lab");
    assert_eq!(block_on(hub.account(&hub_id())).unwrap().username, "ada");
    let nodes = block_on(hub.nodes(&hub_id(), &org)).unwrap();
    assert_eq!(nodes[0].name, "build box");
    let sessions = block_on(hub.sessions(&hub_id(), &org)).unwrap();
    assert_eq!(sessions.len(), 2);
    assert!(
        block_on(hub.sessions(&hub_id(), &OrgId::new("other")))
            .unwrap()
            .is_empty()
    );

    block_on(hub.end(&hub_id(), &HubSessionId::new("s1"))).unwrap();
    let sessions = block_on(hub.sessions(&hub_id(), &org)).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id.as_str(), "s2");
    assert!(matches!(
        block_on(hub.end(&hub_id(), &HubSessionId::new("missing"))),
        Err(HubError::NotFound)
    ));
}

#[test]
fn a_revoked_device_loses_its_sign_in() {
    let hub = signed_in();
    hub.revoke_device(&hub_id());
    assert!(matches!(
        block_on(hub.account(&hub_id())),
        Err(HubError::Revoked)
    ));
    assert!(!hub.is_signed_in(&hub_id()));
    assert!(matches!(
        block_on(hub.account(&hub_id())),
        Err(HubError::NotSignedIn)
    ));
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
    hub.fail_next(FakeOp::Account, HubError::PermissionDenied);
    assert!(matches!(
        block_on(hub.account(&hub_id())),
        Err(HubError::PermissionDenied)
    ));
    assert!(block_on(hub.account(&hub_id())).is_ok());
    assert_eq!(hub.count(FakeOp::Account), 2);
}

#[test]
fn a_held_call_waits_until_released() {
    let hub = signed_in();
    let release = hub.hold(FakeOp::Nodes);
    let mut pending = hub.nodes(&hub_id(), &OrgId::new("lab"));
    block_on(async { assert!(futures::poll!(&mut pending).is_pending()) });
    release.release();
    assert!(block_on(pending).is_ok());
}
