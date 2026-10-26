//! The servers panel registers under its descriptor and the toggle action drives that id.
use super::SERVERS;
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};
use nocterm_ui::contribution_icon;
use nocterm_workspace::test_support::{Toggle, toggle_presentation};

fn registered(
    cx: &mut TestAppContext,
) -> (
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<nocterm_workspace::Workspace>,
) {
    let (window, workspace, _, installed) = crate::test_support::workspace(cx);
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            crate::register(&installed, workspace, window, cx).unwrap();
        });
    })
    .unwrap();
    cx.run_until_parked();
    (window, workspace)
}

#[gpui_kit::test]
fn the_sidebar_toggle_shows_the_descriptors_title_and_icon(cx: &mut TestAppContext) {
    let (_, workspace) = registered(cx);
    cx.update(|cx| {
        let workspace = workspace.read(cx);
        assert!(workspace.is_registered(&SERVERS.id));
        assert_eq!(
            toggle_presentation(workspace, Toggle::Navigation(0), cx),
            Some((SERVERS.title.into(), contribution_icon(SERVERS.icon)))
        );
    });
}

#[gpui_kit::test]
fn the_toggle_action_hides_the_shown_servers_and_shows_them_again(cx: &mut TestAppContext) {
    let (window, workspace) = registered(cx);
    let dispatch = |cx: &mut TestAppContext| {
        cx.update_window(window, |_, window, cx| {
            window.focus(&workspace.read(cx).focus_handle(cx), cx);
            window.dispatch_action(Box::new(crate::ToggleServers), cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let open =
        |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).panel_is_open(&SERVERS.id));
    assert!(open(cx), "the sidebar opens on its first panel");
    dispatch(cx);
    assert!(!open(cx));
    dispatch(cx);
    assert!(open(cx));
}
