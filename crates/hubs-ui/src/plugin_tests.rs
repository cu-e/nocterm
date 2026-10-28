//! The sessions panel registers under its descriptor and waits in the tray of the window.
use super::{PLUGIN, SESSIONS};
use crate::OpenHubSessions;
use crate::test_support::{Fixture, fixture, pinned_record, server};
use gpui_kit::TestAppContext;
use nocterm_hubs_model::testing::FakeHub;

fn window(cx: &mut TestAppContext) -> Fixture {
    fixture(
        cx,
        FakeHub::new().with_server(server()),
        vec![pinned_record()],
        Some("ada"),
    )
}

#[test]
fn the_plugin_is_optional_and_offers_the_sessions_panel() {
    assert!(!PLUGIN.required);
    assert_eq!(PLUGIN.contributions.len(), 1);
    assert_eq!(PLUGIN.contributions[0].id, SESSIONS.id);
}

#[gpui_kit::test]
fn the_window_registers_the_sessions_panel_and_offers_it(cx: &mut TestAppContext) {
    let window = window(cx);
    cx.update(|cx| {
        let workspace = window.workspace.read(cx);
        assert!(workspace.is_registered(&SESSIONS.id));
        assert!(workspace.panel_is_available(&SESSIONS.id));
    });
}

#[gpui_kit::test]
fn opening_the_panel_shows_the_sessions_of_the_signed_in_server(cx: &mut TestAppContext) {
    let window = window(cx);
    window.open_sessions(cx);
    let open = cx.update(|cx| window.workspace.read(cx).panel_is_open(&SESSIONS.id));
    assert!(open);
    assert!(window.shows("hub-session-s1", cx));
    window.open_sessions(cx);
    assert!(window.shows("hub-session-s1", cx), "opening again keeps it");
}

#[gpui_kit::test]
fn the_toggle_action_hides_the_shown_sessions_and_shows_them_again(cx: &mut TestAppContext) {
    let window = window(cx);
    window.open_sessions(cx);
    let open = |cx: &mut TestAppContext| {
        cx.update(|cx| window.workspace.read(cx).panel_is_open(&SESSIONS.id))
    };
    window.dispatch(OpenHubSessions, cx);
    assert!(!open(cx));
    window.dispatch(OpenHubSessions, cx);
    assert!(open(cx));
}
