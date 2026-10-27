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
fn the_window_registers_the_sessions_panel_but_does_not_open_it(cx: &mut TestAppContext) {
    let window = window(cx);
    cx.update(|cx| {
        let workspace = window.workspace.read(cx);
        assert!(workspace.is_registered(&SESSIONS.id));
        assert!(!workspace.panel_is_open(&SESSIONS.id));
    });
}

#[gpui_kit::test]
fn the_toggle_action_shows_the_sessions_and_hides_them_again(cx: &mut TestAppContext) {
    let window = window(cx);
    let open = |cx: &mut TestAppContext| {
        cx.update(|cx| window.workspace.read(cx).panel_is_open(&SESSIONS.id))
    };
    window.dispatch(OpenHubSessions, cx);
    assert!(open(cx));
    window.dispatch(OpenHubSessions, cx);
    assert!(!open(cx));
}
