//! The Hubs commands of the palette pick a known server and act on it.
use crate::test_support::{Fixture, fixture, hub_id, pinned_record, server};
use crate::{OpenServerInNewWindow, Placement, SignInToServer, SignOutOfServer};
use gpui_kit::{AppContext as _, TestAppContext};
use nocterm_hubs::HubStatus;
use nocterm_hubs_model::{Environment, Fingerprint, HubRecord, TrustPolicy, testing::FakeHub};

fn window(cx: &mut TestAppContext, known: Vec<HubRecord>, signed_in: Option<&str>) -> Fixture {
    fixture(
        cx,
        Environment::Local,
        FakeHub::new().with_server(server()),
        known,
        signed_in,
    )
}

fn status(window: &Fixture, cx: &mut TestAppContext) -> Option<HubStatus> {
    cx.update(|cx| window.installed.hubs().read(cx).status(&hub_id()))
}

fn notices(window: &Fixture, cx: &mut TestAppContext) -> usize {
    cx.update_window(window.window, |_, window, cx| {
        nocterm_ui::notice::count(window, cx)
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_chosen_server_opens_in_a_new_window(cx: &mut TestAppContext) {
    let window = window(cx, vec![pinned_record()], Some("ada"));
    window.dispatch(OpenServerInNewWindow, cx);
    assert!(window.has_dialog(cx), "the servers are offered");
    window.press("enter", cx);
    assert_eq!(
        window.environments.requests(),
        vec![(Environment::Hub(hub_id()), Placement::NewWindow)]
    );
}

#[gpui_kit::test]
fn dismissing_the_picker_opens_nothing(cx: &mut TestAppContext) {
    let window = window(cx, vec![pinned_record()], Some("ada"));
    window.dispatch(OpenServerInNewWindow, cx);
    window.press("escape", cx);
    assert!(window.environments.requests().is_empty());
}

#[gpui_kit::test]
fn a_server_that_needs_signing_in_opens_the_sign_in_dialog_instead(cx: &mut TestAppContext) {
    let window = window(cx, vec![pinned_record()], None);
    window.dispatch(OpenServerInNewWindow, cx);
    window.press("enter", cx);
    window.settle_dialog(cx);
    assert!(window.has_dialog(cx));
    assert!(window.environments.requests().is_empty());
}

#[gpui_kit::test]
fn a_changed_certificate_is_explained_and_never_opened(cx: &mut TestAppContext) {
    let changed = HubRecord::new(
        pinned_record().address,
        TrustPolicy::Pinned(Fingerprint::from_bytes([9; 32])),
        "Home",
    );
    let window = window(cx, vec![changed], Some("ada"));
    window.dispatch(OpenServerInNewWindow, cx);
    window.press("enter", cx);
    window.settle_dialog(cx);
    assert!(window.shows("trust-failed-remove", cx));
    assert!(window.environments.requests().is_empty());
}

#[gpui_kit::test]
fn without_known_servers_opening_one_starts_signing_in(cx: &mut TestAppContext) {
    let window = window(cx, Vec::new(), None);
    window.dispatch(OpenServerInNewWindow, cx);
    window.settle_dialog(cx);
    assert!(window.has_dialog(cx));
    assert!(window.environments.requests().is_empty());
}

#[gpui_kit::test]
fn signing_out_of_the_chosen_server_keeps_it_listed(cx: &mut TestAppContext) {
    let window = window(cx, vec![pinned_record()], Some("ada"));
    window.dispatch(SignOutOfServer, cx);
    assert!(window.has_dialog(cx), "the signed-in servers are offered");
    window.press("enter", cx);
    cx.run_until_parked();
    assert_eq!(status(&window, cx), Some(HubStatus::SignedOut));
    assert!(!window.hub.is_signed_in(&hub_id()));
}

#[gpui_kit::test]
fn with_no_signed_in_server_signing_out_says_so(cx: &mut TestAppContext) {
    let window = window(cx, vec![pinned_record()], None);
    window.dispatch(SignOutOfServer, cx);
    assert!(!window.has_dialog(cx), "nothing to choose from");
    assert_eq!(notices(&window, cx), 1);
}

#[gpui_kit::test]
fn the_sign_in_command_still_opens_the_sign_in_dialog(cx: &mut TestAppContext) {
    let window = window(cx, Vec::new(), None);
    window.dispatch(SignInToServer, cx);
    window.settle_dialog(cx);
    assert!(window.has_dialog(cx));
}
