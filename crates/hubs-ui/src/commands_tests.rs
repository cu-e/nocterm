//! The Hubs commands of the palette sign in to a server and pick a known one to sign out of.
use crate::test_support::{Fixture, fixture, hub_id, pinned_record, server};
use crate::{SignInToServer, SignOutOfServer};
use gpui_kit::{AppContext as _, TestAppContext};
use nocterm_hubs::HubStatus;
use nocterm_hubs_model::{HubRecord, testing::FakeHub};

fn window(cx: &mut TestAppContext, known: Vec<HubRecord>, signed_in: Option<&str>) -> Fixture {
    fixture(cx, FakeHub::new().with_server(server()), known, signed_in)
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
