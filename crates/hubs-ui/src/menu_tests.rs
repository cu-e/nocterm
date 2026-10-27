//! The environment menu lists this computer and the known servers, and opens what is chosen.
use super::chip;
use crate::test_support::{Fixture, fixture, hub_id, pinned_record, server};
use crate::{OpenEnvironmentMenu, Placement};
use gpui_kit::{Hsla, TestAppContext, hsla};
use nocterm_hubs::HubStatus;
use nocterm_hubs_model::{Environment, Fingerprint, HubRecord, TrustPolicy, testing::FakeHub};
use std::time::{Duration, Instant};

fn local_window(cx: &mut TestAppContext, known: Vec<HubRecord>) -> Fixture {
    fixture(
        cx,
        Environment::Local,
        FakeHub::new().with_server(server()),
        known,
        Some("ada"),
    )
}

fn open_menu(window: &Fixture, cx: &mut TestAppContext) {
    window.dispatch(OpenEnvironmentMenu, cx);
}

#[gpui_kit::test]
fn without_servers_the_menu_offers_this_computer_and_signing_in(cx: &mut TestAppContext) {
    let window = local_window(cx, Vec::new());
    open_menu(&window, cx);
    assert!(
        window.shows(("environment-row", 0usize), cx),
        "this computer"
    );
    assert!(window.shows(("environment-row", 1usize), cx), "sign in");
    assert!(!window.shows(("environment-row", 2usize), cx));
}

#[gpui_kit::test]
fn each_known_server_gets_a_row_between_this_computer_and_signing_in(cx: &mut TestAppContext) {
    let window = local_window(cx, vec![pinned_record()]);
    open_menu(&window, cx);
    assert!(window.shows(("environment-row", 2usize), cx));
    assert!(!window.shows(("environment-row", 3usize), cx));
}

#[gpui_kit::test]
fn clicking_a_signed_in_server_opens_it_in_this_window(cx: &mut TestAppContext) {
    let window = local_window(cx, vec![pinned_record()]);
    open_menu(&window, cx);
    window.click(("environment-row", 1usize), cx);
    assert_eq!(
        window.environments.requests(),
        vec![(Environment::Hub(hub_id()), Placement::ThisWindow)]
    );
}

#[gpui_kit::test]
fn the_new_window_button_of_a_row_opens_it_beside_this_window(cx: &mut TestAppContext) {
    let window = local_window(cx, vec![pinned_record()]);
    open_menu(&window, cx);
    window.click(("environment-new-window", 1usize), cx);
    assert_eq!(
        window.environments.requests(),
        vec![(Environment::Hub(hub_id()), Placement::NewWindow)]
    );
}

#[gpui_kit::test]
fn enter_opens_the_selected_row_and_ctrl_enter_opens_it_in_a_new_window(cx: &mut TestAppContext) {
    let window = local_window(cx, vec![pinned_record()]);
    open_menu(&window, cx);
    window.press("down", cx);
    window.press("enter", cx);
    assert_eq!(
        window.environments.requests(),
        vec![(Environment::Hub(hub_id()), Placement::ThisWindow)]
    );
    open_menu(&window, cx);
    window.press("down", cx);
    window.press("ctrl-enter", cx);
    assert_eq!(
        window.environments.requests().last(),
        Some(&(Environment::Hub(hub_id()), Placement::NewWindow))
    );
}

#[gpui_kit::test]
fn a_server_that_needs_a_new_certificate_decision_opens_no_window(cx: &mut TestAppContext) {
    let changed = HubRecord::new(
        pinned_record().address,
        TrustPolicy::Pinned(Fingerprint::from_bytes([9; 32])),
        "Home",
    );
    let window = local_window(cx, vec![changed]);
    open_menu(&window, cx);
    window.click(("environment-row", 1usize), cx);
    window.settle_dialog(cx);
    assert!(
        window.has_dialog(cx),
        "the certificate problem is explained"
    );
    assert!(window.shows("trust-failed-close", cx));
    assert!(window.shows("trust-failed-remove", cx));
    assert!(
        window.environments.requests().is_empty(),
        "a changed certificate is never opened, let alone accepted"
    );
}

#[gpui_kit::test]
fn a_signed_out_server_opens_the_sign_in_dialog(cx: &mut TestAppContext) {
    let window = fixture(
        cx,
        Environment::Local,
        FakeHub::new().with_server(server()),
        vec![pinned_record()],
        None,
    );
    open_menu(&window, cx);
    window.click(("environment-row", 1usize), cx);
    window.settle_dialog(cx);
    assert!(window.has_dialog(cx));
    assert!(window.environments.requests().is_empty());
}

#[gpui_kit::test]
fn this_computer_row_opens_nothing_in_a_local_window(cx: &mut TestAppContext) {
    let window = local_window(cx, Vec::new());
    open_menu(&window, cx);
    window.click(("environment-row", 0usize), cx);
    assert!(window.environments.requests().is_empty());
}

fn colors() -> [Hsla; 4] {
    [
        hsla(0.3, 1., 0.5, 1.),
        hsla(0., 0., 0.5, 1.),
        hsla(0.1, 1., 0.5, 1.),
        hsla(0., 1., 0.5, 1.),
    ]
}

fn text(status: &HubStatus, now: Instant) -> String {
    let [good, quiet, warn, bad] = colors();
    chip(status, now, good, quiet, warn, bad).0.to_string()
}

#[test]
fn every_status_has_a_chip_and_an_offline_server_counts_down() {
    let now = Instant::now();
    let fingerprint = Fingerprint::from_bytes([1; 32]);
    assert_eq!(text(&HubStatus::Online, now), "Online");
    assert_eq!(text(&HubStatus::Connecting, now), "Connecting…");
    assert_eq!(text(&HubStatus::SignedOut, now), "Sign-in required");
    assert_eq!(text(&HubStatus::Revoked, now), "Device revoked");
    assert_eq!(
        text(
            &HubStatus::TrustFailed {
                pinned: Some(fingerprint),
                presented: fingerprint
            },
            now
        ),
        "Certificate changed"
    );
    assert_eq!(
        text(&HubStatus::UpgradeRequired { hub: 2, client: 1 }, now),
        "Update Nocterm"
    );
    let offline = HubStatus::Offline {
        retry_at: now + Duration::from_secs(5),
    };
    assert_eq!(text(&offline, now), "Offline · retry in 5 s");
    assert_eq!(
        text(&offline, now + Duration::from_millis(1500)),
        "Offline · retry in 4 s"
    );
    assert_eq!(
        text(&offline, now + Duration::from_secs(9)),
        "Offline · retry in 0 s"
    );
}
