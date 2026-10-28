use std::cell::RefCell;
use std::rc::Rc;

use gpui::{Entity, TestAppContext};
use nocterm_hubs_model::testing::{FakeCall, FakeHub, FakeOp};
use nocterm_hubs_model::{Fingerprint, HubColor, HubError, HubSignIn as _, TrustPolicy};
use nocterm_session::Secret;

use crate::fixture::{ADDRESS, Fixture, fixture, fixture_with_failing_settings, id, server};
use crate::{HubStatus, SignInError, SignInEvent, SignInSession, SignInStep, TrustNote};

fn hub() -> FakeHub {
    FakeHub::new().with_server(server())
}

fn start(cx: &mut TestAppContext, fx: &Fixture) -> Entity<SignInSession> {
    fx.hubs.update(cx, |hubs, cx| hubs.start_sign_in(cx))
}

fn step(cx: &mut TestAppContext, session: &Entity<SignInSession>) -> SignInStep {
    session.read_with(cx, |session, _| session.step().clone())
}

fn submit(cx: &mut TestAppContext, session: &Entity<SignInSession>, address: &str, print: &str) {
    session.update(cx, |session, cx| session.submit_address(address, print, cx));
    cx.run_until_parked();
}

fn events(
    cx: &mut TestAppContext,
    session: &Entity<SignInSession>,
) -> (Rc<RefCell<Vec<SignInEvent>>>, gpui::Subscription) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&seen);
    let subscription = cx.update(|cx| {
        cx.subscribe(session, move |_, event: &SignInEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
    });
    (seen, subscription)
}

fn password(cx: &mut TestAppContext, session: &Entity<SignInSession>, user: &str, secret: &str) {
    session.update(cx, |session, cx| {
        session.sign_in_with_password(user, Secret::new(secret), cx);
    });
    cx.run_until_parked();
}

#[gpui::test]
async fn an_unknown_certificate_waits_for_the_user_then_pins_and_signs_in(cx: &mut TestAppContext) {
    let fx = fixture(cx, hub(), Vec::new());
    let session = start(cx, &fx);
    let revision = cx.read(|cx| fx.settings.revision(cx));
    let (seen, _keep) = events(cx, &session);

    submit(cx, &session, ADDRESS, "");
    let SignInStep::ConfirmFingerprint { fingerprint, .. } = step(cx, &session) else {
        panic!("expected the fingerprint to be confirmed");
    };
    assert_eq!(fingerprint, Fingerprint::from_bytes([7; 32]));
    assert_eq!(
        fx.hub.count(FakeOp::Connect),
        0,
        "nothing is sent before the user trusts it"
    );

    session.update(cx, |session, cx| session.confirm_fingerprint(cx));
    cx.run_until_parked();
    let SignInStep::Method {
        note,
        password: offered,
        providers,
        hub_name,
        ..
    } = step(cx, &session)
    else {
        panic!("expected the sign-in methods");
    };
    assert_eq!(note, TrustNote::Confirmed);
    assert!(offered);
    assert_eq!(providers.len(), 1);
    assert_eq!(hub_name, "Home hub");
    assert!(
        fx.servers(cx).is_empty(),
        "the server is not listed before signing in"
    );

    password(cx, &session, "ada", "secret");
    assert!(matches!(step(cx, &session), SignInStep::Finished(hub) if hub == id()));
    assert_eq!(*seen.borrow(), [SignInEvent::Finished(id())]);

    let servers = fx.servers(cx);
    assert_eq!(servers.len(), 1);
    assert_eq!(
        servers[0].trust,
        TrustPolicy::Pinned(Fingerprint::from_bytes([7; 32]))
    );
    assert_eq!(servers[0].label, "Home hub");
    assert_eq!(
        cx.read(|cx| fx.settings.revision(cx)),
        revision + 1,
        "one edit"
    );
    assert_eq!(fx.status(cx), Some(HubStatus::Online));
}

#[gpui::test]
async fn a_typed_fingerprint_that_matches_skips_the_confirmation(cx: &mut TestAppContext) {
    let fx = fixture(cx, hub(), Vec::new());
    let session = start(cx, &fx);
    submit(
        cx,
        &session,
        &format!("https://{ADDRESS}"),
        &Fingerprint::from_bytes([7; 32]).to_string(),
    );
    assert!(matches!(
        step(cx, &session),
        SignInStep::Method {
            note: TrustNote::Pinned,
            ..
        }
    ));
}

#[gpui::test]
async fn a_typed_fingerprint_that_differs_is_refused_before_any_credentials_are_sent(
    cx: &mut TestAppContext,
) {
    let fx = fixture(cx, hub(), Vec::new());
    let session = start(cx, &fx);
    submit(
        cx,
        &session,
        ADDRESS,
        &Fingerprint::from_bytes([1; 32]).to_string(),
    );
    assert!(matches!(
        step(cx, &session),
        SignInStep::Mismatch { typed, presented }
            if typed == Fingerprint::from_bytes([1; 32]) && presented == Fingerprint::from_bytes([7; 32])
    ));
    assert_eq!(fx.hub.count(FakeOp::Connect), 0);
    session.update(cx, |session, cx| session.back(cx));
    assert!(matches!(
        step(cx, &session),
        SignInStep::Address { problem: None }
    ));
}

#[gpui::test]
async fn a_publicly_trusted_certificate_needs_no_confirmation(cx: &mut TestAppContext) {
    let hub = FakeHub::new().with_server(server().publicly_trusted());
    let fx = fixture(cx, hub, Vec::new());
    let session = start(cx, &fx);
    submit(cx, &session, ADDRESS, "");
    assert!(matches!(
        step(cx, &session),
        SignInStep::Method {
            note: TrustNote::PublicCa,
            ..
        }
    ));
    password(cx, &session, "ada", "secret");
    assert_eq!(fx.servers(cx)[0].trust, TrustPolicy::PublicCa);
}

#[gpui::test]
async fn a_malformed_address_or_fingerprint_is_reported_on_the_address_step(
    cx: &mut TestAppContext,
) {
    let fx = fixture(cx, hub(), Vec::new());
    let session = start(cx, &fx);
    submit(cx, &session, "http://hub.home", "");
    assert!(matches!(
        step(cx, &session),
        SignInStep::Address {
            problem: Some(SignInError::Address(_))
        }
    ));
    submit(cx, &session, ADDRESS, "xyz");
    assert!(matches!(
        step(cx, &session),
        SignInStep::Address {
            problem: Some(SignInError::Fingerprint(_))
        }
    ));
    assert_eq!(fx.hub.calls(), []);
}

#[gpui::test]
async fn an_unreachable_server_returns_to_the_address_step(cx: &mut TestAppContext) {
    let fx = fixture(cx, hub(), Vec::new());
    let session = start(cx, &fx);
    submit(cx, &session, "nowhere.example", "");
    assert!(matches!(
        step(cx, &session),
        SignInStep::Address {
            problem: Some(SignInError::Hub(HubError::Unreachable { .. }))
        }
    ));
}

#[gpui::test]
async fn a_server_without_an_owner_cannot_be_signed_in_to(cx: &mut TestAppContext) {
    let hub = hub();
    hub.edit_server(&crate::fixture::address(), |server| {
        server.bootstrapped = false
    });
    let fx = fixture(cx, hub, Vec::new());
    let session = start(cx, &fx);
    submit(
        cx,
        &session,
        ADDRESS,
        &Fingerprint::from_bytes([7; 32]).to_string(),
    );
    assert!(matches!(
        step(cx, &session),
        SignInStep::Address {
            problem: Some(SignInError::NotInitialized)
        }
    ));
}

#[gpui::test]
async fn a_wrong_password_returns_to_the_methods_and_changes_nothing(cx: &mut TestAppContext) {
    let fx = fixture(cx, hub(), Vec::new());
    let session = start(cx, &fx);
    let revision = cx.read(|cx| fx.settings.revision(cx));
    submit(
        cx,
        &session,
        ADDRESS,
        &Fingerprint::from_bytes([7; 32]).to_string(),
    );
    password(cx, &session, "ada", "wrong");
    assert!(matches!(
        step(cx, &session),
        SignInStep::Method {
            problem: Some(SignInError::Hub(HubError::InvalidCredentials)),
            ..
        }
    ));
    assert!(fx.servers(cx).is_empty());
    assert_eq!(cx.read(|cx| fx.settings.revision(cx)), revision);
    password(cx, &session, "ada", "secret");
    assert!(matches!(step(cx, &session), SignInStep::Finished(_)));
}

#[gpui::test]
async fn a_device_that_is_already_signed_in_is_listed_without_asking_for_a_password(
    cx: &mut TestAppContext,
) {
    let hub = hub();
    hub.sign_in_as(&id(), "ada");
    let fx = fixture(cx, hub, Vec::new());
    let session = start(cx, &fx);
    submit(
        cx,
        &session,
        ADDRESS,
        &Fingerprint::from_bytes([7; 32]).to_string(),
    );
    assert!(matches!(step(cx, &session), SignInStep::Finished(_)));
    assert_eq!(fx.hub.count(FakeOp::Password), 0);
    assert_eq!(fx.servers(cx).len(), 1);
}

#[gpui::test]
async fn the_browser_sign_in_opens_the_address_the_hub_gives_and_finishes_in_the_browser(
    cx: &mut TestAppContext,
) {
    let fx = fixture(cx, hub(), Vec::new());
    let session = start(cx, &fx);
    let (seen, _keep) = events(cx, &session);
    submit(
        cx,
        &session,
        ADDRESS,
        &Fingerprint::from_bytes([7; 32]).to_string(),
    );
    session.update(cx, |session, cx| session.sign_in_with_provider("corp", cx));
    cx.run_until_parked();
    let opened = seen.borrow().clone();
    assert_eq!(opened.len(), 1);
    let SignInEvent::OpenBrowser(url) = &opened[0] else {
        panic!("expected the browser to be opened");
    };
    assert!(url.contains("corp"));
    assert!(matches!(
        step(cx, &session),
        SignInStep::Browser { url: Some(shown), .. } if shown == *url
    ));

    fx.hub.complete_oidc("ada");
    cx.run_until_parked();
    assert!(matches!(step(cx, &session), SignInStep::Finished(_)));
    assert_eq!(fx.servers(cx).len(), 1);
}

#[gpui::test]
async fn going_back_from_the_browser_cancels_the_sign_in(cx: &mut TestAppContext) {
    let fx = fixture(cx, hub(), Vec::new());
    let session = start(cx, &fx);
    submit(
        cx,
        &session,
        ADDRESS,
        &Fingerprint::from_bytes([7; 32]).to_string(),
    );
    session.update(cx, |session, cx| session.sign_in_with_provider("corp", cx));
    cx.run_until_parked();
    session.update(cx, |session, cx| session.back(cx));
    cx.run_until_parked();
    assert!(matches!(
        step(cx, &session),
        SignInStep::Method { problem: None, .. }
    ));
    assert_eq!(fx.hub.cancelled_oidc(), 1);
    assert!(fx.servers(cx).is_empty());
}

#[gpui::test]
async fn a_browser_sign_in_that_fails_returns_to_the_methods(cx: &mut TestAppContext) {
    let fx = fixture(cx, hub(), Vec::new());
    let session = start(cx, &fx);
    submit(
        cx,
        &session,
        ADDRESS,
        &Fingerprint::from_bytes([7; 32]).to_string(),
    );
    session.update(cx, |session, cx| session.sign_in_with_provider("corp", cx));
    cx.run_until_parked();
    fx.hub.fail_oidc(HubError::SignInTimedOut);
    cx.run_until_parked();
    assert!(matches!(
        step(cx, &session),
        SignInStep::Method {
            problem: Some(SignInError::Hub(HubError::SignInTimedOut)),
            ..
        }
    ));
}

#[gpui::test]
async fn credentials_made_for_a_server_that_could_not_be_saved_are_erased(cx: &mut TestAppContext) {
    let fx = fixture_with_failing_settings(cx, hub());
    let session = start(cx, &fx);
    submit(
        cx,
        &session,
        ADDRESS,
        &Fingerprint::from_bytes([7; 32]).to_string(),
    );
    password(cx, &session, "ada", "secret");
    cx.run_until_parked();
    assert!(matches!(
        step(cx, &session),
        SignInStep::Failed(SignInError::Save(_))
    ));
    assert!(!fx.hub.is_signed_in(&id()));
    assert!(fx.hub.calls().contains(&FakeCall::Forget(id())));
    assert!(fx.servers(cx).is_empty());
}

#[gpui::test]
async fn signing_in_again_to_a_revoked_server_starts_at_the_methods(cx: &mut TestAppContext) {
    let hub = hub();
    hub.sign_in_as(&id(), "ada");
    hub.revoke_device(&id());
    let fx = fixture(cx, hub, vec![crate::fixture::pinned_record()]);
    cx.run_until_parked();
    assert_eq!(fx.status(cx), Some(HubStatus::Revoked));

    let session = fx
        .hubs
        .update(cx, |hubs, cx| hubs.start_sign_in_for(&id(), cx))
        .expect("a known hub");
    cx.run_until_parked();
    assert!(matches!(
        step(cx, &session),
        SignInStep::Method {
            note: TrustNote::Pinned,
            ..
        }
    ));
    assert!(!session.read_with(cx, |session, _| session.address_editable()));
    session.update(cx, |session, cx| session.back(cx));
    assert!(
        matches!(step(cx, &session), SignInStep::Method { .. }),
        "the address stays fixed"
    );

    password(cx, &session, "ada", "secret");
    assert!(matches!(step(cx, &session), SignInStep::Finished(_)));
    assert_eq!(fx.servers(cx)[0].label, "Home", "a known label is kept");
    cx.run_until_parked();
    assert_eq!(fx.status(cx), Some(HubStatus::Online));
}

#[gpui::test]
async fn a_known_address_is_refused_so_a_changed_certificate_is_never_pinned_again(
    cx: &mut TestAppContext,
) {
    let hub = hub();
    let fx = fixture(cx, hub, vec![crate::fixture::pinned_record()]);
    cx.run_until_parked();
    let pinned = fx.servers(cx);
    fx.hub.edit_server(&crate::fixture::address(), |server| {
        server.certificate = Fingerprint::from_bytes([9; 32]);
    });
    let probes = fx.hub.count(FakeOp::Probe);

    let session = start(cx, &fx);
    submit(cx, &session, ADDRESS, "");

    assert!(
        matches!(
            step(cx, &session),
            SignInStep::Address {
                problem: Some(SignInError::AlreadyKnown(hub))
            } if hub == id()
        ),
        "the dialog does not offer to trust the new certificate"
    );
    assert_eq!(fx.hub.count(FakeOp::Probe), probes, "nothing was asked");
    assert_eq!(fx.servers(cx), pinned, "the pinned record is unchanged");
}

#[gpui::test]
async fn a_resumed_sign_in_cannot_be_pointed_at_another_server(cx: &mut TestAppContext) {
    let hub = hub();
    let fx = fixture(cx, hub, vec![crate::fixture::pinned_record()]);
    cx.run_until_parked();
    fx.hub
        .edit_server(&crate::fixture::address(), |server| server.online = false);
    let session = fx
        .hubs
        .update(cx, |hubs, cx| hubs.start_sign_in_for(&id(), cx))
        .expect("a known hub");
    cx.run_until_parked();
    assert!(matches!(step(cx, &session), SignInStep::Address { .. }));
    let (probes, connects) = (fx.hub.count(FakeOp::Probe), fx.hub.count(FakeOp::Connect));

    submit(cx, &session, "other.example:7443", "");

    assert!(matches!(step(cx, &session), SignInStep::Address { .. }));
    assert_eq!(fx.hub.count(FakeOp::Probe), probes);
    assert_eq!(fx.hub.count(FakeOp::Connect), connects);
}

#[gpui::test]
async fn signing_in_again_keeps_the_colour_chosen_for_the_server(cx: &mut TestAppContext) {
    let hub = hub();
    hub.sign_in_as(&id(), "ada");
    hub.revoke_device(&id());
    let marked = crate::fixture::pinned_record().with_color(Some(HubColor::Blue));
    let fx = fixture(cx, hub, vec![marked]);
    cx.run_until_parked();

    let session = fx
        .hubs
        .update(cx, |hubs, cx| hubs.start_sign_in_for(&id(), cx))
        .expect("a known hub");
    cx.run_until_parked();
    password(cx, &session, "ada", "secret");

    assert!(matches!(step(cx, &session), SignInStep::Finished(_)));
    assert_eq!(fx.servers(cx)[0].color, Some(HubColor::Blue));
}

fn keyed_hub() -> FakeHub {
    FakeHub::new().with_server(server().keyed())
}

fn at_methods(cx: &mut TestAppContext, fx: &Fixture) -> Entity<SignInSession> {
    let session = start(cx, fx);
    submit(
        cx,
        &session,
        ADDRESS,
        &Fingerprint::from_bytes([7; 32]).to_string(),
    );
    session
}

#[gpui::test]
async fn a_first_sign_in_shows_the_recovery_key_once_and_lists_the_server_when_it_is_acknowledged(
    cx: &mut TestAppContext,
) {
    let fx = fixture(cx, keyed_hub(), Vec::new());
    let session = at_methods(cx, &fx);
    password(cx, &session, "ada", "secret");

    let SignInStep::RecoveryKey { key } = step(cx, &session) else {
        panic!("expected the recovery key");
    };
    assert_eq!(key, Secret::new("recovery-ada"));
    assert!(
        fx.servers(cx).is_empty(),
        "the server waits for the key to be written down"
    );

    session.update(cx, |session, cx| session.acknowledge_recovery_key(cx));
    cx.run_until_parked();
    assert!(matches!(step(cx, &session), SignInStep::Finished(_)));
    assert_eq!(fx.servers(cx).len(), 1);
}

#[gpui::test]
async fn a_reset_password_asks_for_the_recovery_key_and_refuses_a_wrong_one(
    cx: &mut TestAppContext,
) {
    let hub = FakeHub::new().with_server(server().with_keyset("ada", "old", "recovery-key"));
    let fx = fixture(cx, hub, Vec::new());
    let session = at_methods(cx, &fx);
    password(cx, &session, "ada", "secret");
    assert!(matches!(
        step(cx, &session),
        SignInStep::Recovery { problem: None }
    ));

    session.update(cx, |session, cx| {
        session.recover(Secret::new("not it"), cx);
    });
    cx.run_until_parked();
    assert!(matches!(
        step(cx, &session),
        SignInStep::Recovery {
            problem: Some(SignInError::Hub(HubError::RecoveryKeyInvalid))
        }
    ));
    assert!(fx.servers(cx).is_empty());

    session.update(cx, |session, cx| {
        session.recover(Secret::new("recovery-key"), cx);
    });
    cx.run_until_parked();
    assert!(matches!(step(cx, &session), SignInStep::Finished(_)));
    assert_eq!(fx.servers(cx).len(), 1);
}

#[gpui::test]
async fn a_browser_sign_in_asks_for_a_vault_passphrase_before_listing_the_server(
    cx: &mut TestAppContext,
) {
    let fx = fixture(cx, keyed_hub(), Vec::new());
    let session = at_methods(cx, &fx);
    session.update(cx, |session, cx| session.sign_in_with_provider("corp", cx));
    cx.run_until_parked();
    fx.hub.complete_oidc("ada");
    cx.run_until_parked();
    assert!(matches!(
        step(cx, &session),
        SignInStep::Passphrase { problem: None }
    ));

    session.update(cx, |session, cx| {
        session.set_vault_passphrase(Secret::new("a long passphrase"), cx);
    });
    cx.run_until_parked();
    assert!(matches!(step(cx, &session), SignInStep::RecoveryKey { .. }));
    session.update(cx, |session, cx| session.acknowledge_recovery_key(cx));
    cx.run_until_parked();
    assert!(matches!(step(cx, &session), SignInStep::Finished(_)));
}

#[gpui::test]
async fn the_key_steps_do_nothing_out_of_turn(cx: &mut TestAppContext) {
    let fx = fixture(cx, keyed_hub(), Vec::new());
    let session = at_methods(cx, &fx);
    session.update(cx, |session, cx| {
        session.acknowledge_recovery_key(cx);
        session.issue_recovery_key(cx);
        session.skip_recovery_offer(cx);
        session.recover(Secret::new("recovery-ada"), cx);
        session.set_vault_passphrase(Secret::new("a long passphrase"), cx);
    });
    cx.run_until_parked();
    assert!(matches!(step(cx, &session), SignInStep::Method { .. }));
    assert_eq!(fx.hub.count(FakeOp::Recover), 0);
    assert_eq!(fx.hub.count(FakeOp::ReissueRecoveryKey), 0);
    assert_eq!(fx.hub.count(FakeOp::SetVaultPassphrase), 0);
}

/// The sign-in of a first account, closed on the recovery-key step: the server is not listed and
/// the key was shown but never confirmed.
fn closed_on_the_recovery_key(cx: &mut TestAppContext, fx: &Fixture) {
    let session = at_methods(cx, fx);
    password(cx, &session, "ada", "secret");
    assert!(matches!(step(cx, &session), SignInStep::RecoveryKey { .. }));
    drop(session);
    cx.run_until_parked();
}

fn issue(cx: &mut TestAppContext, session: &Entity<SignInSession>) {
    session.update(cx, |session, cx| session.issue_recovery_key(cx));
    cx.run_until_parked();
}

#[gpui::test]
async fn acknowledging_the_recovery_key_confirms_it_so_it_is_not_offered_again(
    cx: &mut TestAppContext,
) {
    let fx = fixture(cx, keyed_hub(), Vec::new());
    let session = at_methods(cx, &fx);
    password(cx, &session, "ada", "secret");
    assert!(fx.hub.recovery_key_unconfirmed(&id()));

    session.update(cx, |session, cx| session.acknowledge_recovery_key(cx));
    cx.run_until_parked();

    assert!(!fx.hub.recovery_key_unconfirmed(&id()));
}

#[gpui::test]
async fn a_sign_in_closed_on_the_recovery_key_offers_a_new_one_next_time(cx: &mut TestAppContext) {
    let fx = fixture(cx, keyed_hub(), Vec::new());
    closed_on_the_recovery_key(cx, &fx);
    assert!(fx.servers(cx).is_empty());

    let session = start(cx, &fx);
    submit(
        cx,
        &session,
        ADDRESS,
        &Fingerprint::from_bytes([7; 32]).to_string(),
    );
    assert!(matches!(
        step(cx, &session),
        SignInStep::RecoveryOffer { problem: None }
    ));
    assert!(
        fx.servers(cx).is_empty(),
        "nothing is listed before it is settled"
    );

    issue(cx, &session);
    let SignInStep::RecoveryKey { key } = step(cx, &session) else {
        panic!("expected the new recovery key");
    };
    assert_eq!(
        key,
        Secret::new("recovery-ada+"),
        "the old key was replaced"
    );
    session.update(cx, |session, cx| session.acknowledge_recovery_key(cx));
    cx.run_until_parked();
    assert!(matches!(step(cx, &session), SignInStep::Finished(_)));
    assert_eq!(fx.servers(cx).len(), 1);
    assert!(!fx.hub.recovery_key_unconfirmed(&id()));
}

#[gpui::test]
async fn a_password_sign_in_that_finds_the_key_unconfirmed_offers_a_new_one(
    cx: &mut TestAppContext,
) {
    let fx = fixture(cx, keyed_hub(), Vec::new());
    closed_on_the_recovery_key(cx, &fx);
    fx.hub.expire_login(&id());

    let session = at_methods(cx, &fx);
    password(cx, &session, "ada", "secret");

    assert!(matches!(
        step(cx, &session),
        SignInStep::RecoveryOffer { problem: None }
    ));
}

#[gpui::test]
async fn leaving_the_offer_for_later_lists_the_server_and_keeps_the_key_unconfirmed(
    cx: &mut TestAppContext,
) {
    let fx = fixture(cx, keyed_hub(), Vec::new());
    closed_on_the_recovery_key(cx, &fx);
    let session = start(cx, &fx);
    submit(
        cx,
        &session,
        ADDRESS,
        &Fingerprint::from_bytes([7; 32]).to_string(),
    );

    session.update(cx, |session, cx| session.skip_recovery_offer(cx));
    cx.run_until_parked();

    assert!(matches!(step(cx, &session), SignInStep::Finished(_)));
    assert_eq!(fx.servers(cx).len(), 1);
    assert!(
        fx.hubs
            .read_with(cx, |hubs, _| hubs.recovery_key_unconfirmed(&id()))
    );
}

#[gpui::test]
async fn a_failed_reissue_returns_to_the_offer_and_can_be_tried_again(cx: &mut TestAppContext) {
    let fx = fixture(cx, keyed_hub(), Vec::new());
    closed_on_the_recovery_key(cx, &fx);
    let session = start(cx, &fx);
    submit(
        cx,
        &session,
        ADDRESS,
        &Fingerprint::from_bytes([7; 32]).to_string(),
    );
    fx.hub
        .fail_next(FakeOp::ReissueRecoveryKey, HubError::KeysLocked);

    issue(cx, &session);
    assert!(matches!(
        step(cx, &session),
        SignInStep::RecoveryOffer {
            problem: Some(SignInError::Hub(HubError::KeysLocked))
        }
    ));

    issue(cx, &session);
    assert!(matches!(step(cx, &session), SignInStep::RecoveryKey { .. }));
}

#[gpui::test]
async fn the_servers_page_can_issue_a_new_key_for_a_listed_server(cx: &mut TestAppContext) {
    let fx = fixture(cx, keyed_hub(), Vec::new());
    closed_on_the_recovery_key(cx, &fx);
    let first = start(cx, &fx);
    submit(
        cx,
        &first,
        ADDRESS,
        &Fingerprint::from_bytes([7; 32]).to_string(),
    );
    // Listed, with the key shown but not confirmed.
    first.update(cx, |session, cx| session.skip_recovery_offer(cx));
    cx.run_until_parked();
    assert_eq!(fx.servers(cx).len(), 1);
    assert!(
        fx.hubs
            .read_with(cx, |hubs, _| hubs.recovery_key_unconfirmed(&id()))
    );

    let session = fx
        .hubs
        .update(cx, |hubs, cx| hubs.start_recovery_key_reissue(&id(), cx))
        .expect("a listed server");
    cx.run_until_parked();
    assert!(session.read_with(cx, |session, _| session.reissue_only()));
    let (seen, _keep) = events(cx, &session);
    let SignInStep::RecoveryKey { key } = step(cx, &session) else {
        panic!("expected the new recovery key");
    };
    assert_eq!(key, Secret::new("recovery-ada+"));

    session.update(cx, |session, cx| session.acknowledge_recovery_key(cx));
    cx.run_until_parked();
    assert!(matches!(step(cx, &session), SignInStep::Finished(_)));
    assert_eq!(*seen.borrow(), vec![SignInEvent::Finished(id())]);
    assert!(
        !fx.hubs
            .read_with(cx, |hubs, _| hubs.recovery_key_unconfirmed(&id()))
    );
}

#[gpui::test]
async fn no_key_is_issued_for_a_server_that_is_not_listed(cx: &mut TestAppContext) {
    let fx = fixture(cx, keyed_hub(), Vec::new());
    let session = fx
        .hubs
        .update(cx, |hubs, cx| hubs.start_recovery_key_reissue(&id(), cx));
    assert!(session.is_none());
    assert_eq!(fx.hub.count(FakeOp::ReissueRecoveryKey), 0);
}
