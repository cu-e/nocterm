//! The sessions panel registers under its descriptor, in the window of a hub only.
use super::SESSIONS;
use crate::OpenHubSessions;
use crate::test_support::{Fixture, fixture, hub_id, pinned_record, server};
use gpui_kit::TestAppContext;
use nocterm_hubs_model::{Environment, testing::FakeHub};
use nocterm_ui::contribution_icon;
use nocterm_workspace::test_support::{Toggle, toggle_presentation};

fn window_of(environment: Environment, cx: &mut TestAppContext) -> Fixture {
    fixture(
        cx,
        environment,
        FakeHub::new().with_server(server()),
        vec![pinned_record()],
        Some("ada"),
    )
}

#[gpui_kit::test]
fn a_hub_window_registers_the_sessions_panel_with_its_title_and_icon(cx: &mut TestAppContext) {
    let window = window_of(Environment::Hub(hub_id()), cx);
    cx.update(|cx| {
        let workspace = window.workspace.read(cx);
        assert!(workspace.is_registered(&SESSIONS.id));
        assert_eq!(
            toggle_presentation(workspace, Toggle::Navigation(0), cx),
            Some((SESSIONS.title.into(), contribution_icon(SESSIONS.icon)))
        );
    });
}

#[gpui_kit::test]
fn a_local_window_has_no_sessions_panel(cx: &mut TestAppContext) {
    let window = window_of(Environment::Local, cx);
    cx.update(|cx| {
        assert!(!window.workspace.read(cx).is_registered(&SESSIONS.id));
    });
}

#[gpui_kit::test]
fn the_toggle_action_hides_the_shown_sessions_and_shows_them_again(cx: &mut TestAppContext) {
    let window = window_of(Environment::Hub(hub_id()), cx);
    let open = |cx: &mut TestAppContext| {
        cx.update(|cx| window.workspace.read(cx).panel_is_open(&SESSIONS.id))
    };
    assert!(open(cx), "the sidebar opens on its first panel");
    window.dispatch(OpenHubSessions, cx);
    assert!(!open(cx));
    window.dispatch(OpenHubSessions, cx);
    assert!(open(cx));
}
