//! Signing in to a server through the dialog, step by step, over a fake hub.
use crate::SignInToServer;
use crate::test_support::{Fixture, fixture, server};
use gpui_kit::TestAppContext;
use nocterm_hubs_model::testing::{FakeCall, FakeHub, FakeOp, FakeServer};
use nocterm_hubs_model::{Fingerprint, HubError};

fn dialog_over(server: FakeServer, cx: &mut TestAppContext) -> Fixture {
    let window = fixture(cx, FakeHub::new().with_server(server), Vec::new(), None);
    window.dispatch(SignInToServer, cx);
    window.settle_dialog(cx);
    assert!(window.has_dialog(cx));
    window
}

/// Types the address (and a fingerprint) and moves on.
fn submit(
    window: &Fixture,
    address: &str,
    fingerprint: Option<&Fingerprint>,
    cx: &mut TestAppContext,
) {
    window.type_text(address, cx);
    if let Some(fingerprint) = fingerprint {
        window.press("tab", cx);
        window.type_text(&fingerprint.to_string(), cx);
    }
    window.click("sign-in-continue", cx);
    cx.run_until_parked();
    window.render(cx);
}

fn sign_in_with_password(window: &Fixture, user: &str, password: &str, cx: &mut TestAppContext) {
    window.type_text(user, cx);
    window.press("tab", cx);
    window.type_text(password, cx);
    window.press("enter", cx);
    cx.run_until_parked();
    window.render(cx);
}

fn saved(window: &Fixture, cx: &mut TestAppContext) -> usize {
    cx.update(|cx| window.installed.hubs().read(cx).list().len())
}

#[gpui_kit::test]
fn a_matching_fingerprint_goes_straight_to_the_password(cx: &mut TestAppContext) {
    let window = dialog_over(server(), cx);
    submit(
        &window,
        "hub.home:7443",
        Some(&Fingerprint::from_bytes([7; 32])),
        cx,
    );
    assert!(window.shows("sign-in-password", cx));
    assert!(!window.shows("sign-in-trust", cx));
}

#[gpui_kit::test]
fn a_fingerprint_that_does_not_match_offers_only_going_back(cx: &mut TestAppContext) {
    let window = dialog_over(server(), cx);
    submit(
        &window,
        "hub.home:7443",
        Some(&Fingerprint::from_bytes([8; 32])),
        cx,
    );
    assert!(window.shows("sign-in-back", cx));
    assert!(!window.shows("sign-in-trust", cx));
    assert!(!window.shows("sign-in-password", cx));
    assert!(!window.shows("sign-in-continue", cx));
    assert_eq!(
        window.hub.count(FakeOp::Connect),
        0,
        "the hub is not contacted again"
    );
    assert_eq!(saved(&window, cx), 0);
}

#[gpui_kit::test]
fn a_self_signed_server_asks_to_trust_and_pin_its_certificate(cx: &mut TestAppContext) {
    let window = dialog_over(server(), cx);
    submit(&window, "hub.home:7443", None, cx);
    assert!(window.shows("sign-in-trust", cx));
    assert!(window.shows("sign-in-back", cx));
    window.click("sign-in-trust", cx);
    cx.run_until_parked();
    window.render(cx);
    assert!(window.shows("sign-in-password", cx));
}

#[gpui_kit::test]
fn enter_on_the_trust_step_goes_back_instead_of_trusting(cx: &mut TestAppContext) {
    let window = dialog_over(server(), cx);
    submit(&window, "hub.home:7443", None, cx);
    window.press("enter", cx);
    assert!(window.shows("sign-in-continue", cx), "back at the address");
    assert!(!window.shows("sign-in-password", cx));
    assert_eq!(window.hub.count(FakeOp::Connect), 0);
}

#[gpui_kit::test]
fn a_certificate_from_a_public_authority_needs_no_decision(cx: &mut TestAppContext) {
    let window = dialog_over(server().publicly_trusted(), cx);
    submit(&window, "hub.home:7443", None, cx);
    assert!(window.shows("sign-in-password", cx));
    assert!(!window.shows("sign-in-trust", cx));
}

#[gpui_kit::test]
fn a_wrong_password_empties_the_field_and_the_right_one_signs_in(cx: &mut TestAppContext) {
    let window = dialog_over(server().publicly_trusted(), cx);
    submit(&window, "hub.home:7443", None, cx);
    sign_in_with_password(&window, "ada", "wrong", cx);
    assert!(window.shows("sign-in-password", cx), "back at the methods");
    assert!(!window.shows("sign-in-done", cx));
    // Had the wrong password stayed in its field, this would be "wrongsecret".
    window.press("tab", cx);
    window.type_text("secret", cx);
    window.press("enter", cx);
    cx.run_until_parked();
    window.render(cx);
    assert!(window.shows("sign-in-done", cx));
    assert_eq!(saved(&window, cx), 1, "the server is saved");
    let attempts = window
        .hub
        .calls()
        .iter()
        .filter(|call| matches!(call, FakeCall::Password { .. }))
        .count();
    assert_eq!(attempts, 2);
}

#[gpui_kit::test]
fn done_closes_the_dialog_and_the_server_is_listed(cx: &mut TestAppContext) {
    let window = dialog_over(server().publicly_trusted(), cx);
    submit(&window, "hub.home:7443", None, cx);
    sign_in_with_password(&window, "ada", "secret", cx);
    window.click("sign-in-done", cx);
    assert!(!window.has_dialog(cx));
    assert_eq!(saved(&window, cx), 1);
}

#[gpui_kit::test]
fn browser_sign_in_opens_the_provider_page_and_finishes_when_the_browser_does(
    cx: &mut TestAppContext,
) {
    let window = dialog_over(server().publicly_trusted(), cx);
    submit(&window, "hub.home:7443", None, cx);
    window.click("sign-in-provider-corp", cx);
    cx.run_until_parked();
    window.render(cx);
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://idp.fake/authorize?provider=corp")
    );
    assert!(window.shows("sign-in-copy-link", cx));
    window.hub.complete_oidc("ada");
    cx.run_until_parked();
    window.render(cx);
    assert!(window.shows("sign-in-done", cx));
}

#[gpui_kit::test]
fn cancelling_the_browser_sign_in_stops_waiting_for_it(cx: &mut TestAppContext) {
    let window = dialog_over(server().publicly_trusted(), cx);
    submit(&window, "hub.home:7443", None, cx);
    window.click("sign-in-provider-corp", cx);
    cx.run_until_parked();
    window.render(cx);
    window.click("sign-in-stop", cx);
    cx.run_until_parked();
    assert_eq!(window.hub.cancelled_oidc(), 1);
    assert!(window.shows("sign-in-password", cx), "back at the methods");
    assert_eq!(saved(&window, cx), 0);
}

#[gpui_kit::test]
fn closing_the_dialog_during_a_browser_sign_in_cancels_it(cx: &mut TestAppContext) {
    let window = dialog_over(server().publicly_trusted(), cx);
    submit(&window, "hub.home:7443", None, cx);
    window.click("sign-in-provider-corp", cx);
    cx.run_until_parked();
    window.render(cx);
    window.press("escape", cx);
    cx.run_until_parked();
    assert!(!window.has_dialog(cx));
    assert_eq!(window.hub.cancelled_oidc(), 1);
}

#[gpui_kit::test]
fn a_server_that_speaks_another_protocol_is_refused_at_the_address(cx: &mut TestAppContext) {
    let mut outdated = server().publicly_trusted();
    outdated.protocol = 9;
    let window = dialog_over(outdated, cx);
    submit(&window, "hub.home:7443", None, cx);
    assert!(window.shows("sign-in-continue", cx), "still at the address");
    assert!(!window.shows("sign-in-password", cx));
    assert_eq!(saved(&window, cx), 0);
}

#[gpui_kit::test]
fn a_server_that_does_not_answer_leaves_the_address_to_correct(cx: &mut TestAppContext) {
    let window = dialog_over(server(), cx);
    window.hub.fail_next(
        FakeOp::Probe,
        HubError::Unreachable {
            address: nocterm_hubs_model::HubAddress::parse("hub.home:7443").unwrap(),
            cause: nocterm_core::error::SharedCause::new(std::io::Error::from(
                std::io::ErrorKind::ConnectionRefused,
            )),
        },
    );
    submit(&window, "hub.home:7443", None, cx);
    assert!(window.shows("sign-in-continue", cx));
}

#[gpui_kit::test]
fn the_address_is_validated_before_the_hub_is_asked(cx: &mut TestAppContext) {
    let window = dialog_over(server(), cx);
    submit(&window, "http://hub.home", None, cx);
    assert!(window.shows("sign-in-continue", cx));
    assert_eq!(window.hub.count(FakeOp::Probe), 0);
}
