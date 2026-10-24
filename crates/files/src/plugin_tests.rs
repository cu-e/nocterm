//! The Explorer registers under its descriptors and the toggle action drives that id.
use super::*;
use crate::plugin::{EXPLORER, TRANSFERS};
use gpui_kit::{AppContext as _, Focusable as _};
use nocterm_ui::contribution_icon;
use nocterm_workspace::test_support::{Toggle, toggle_presentation};

fn registered(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
    let (window, workspace, _, _, installed) =
        panel_fixture_with_installation(cx, Arc::new(FakeFs), fixture_deps());
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
        assert!(workspace.is_registered(&EXPLORER.id));
        assert!(workspace.is_registered(&TRANSFERS.id));
        assert_eq!(
            toggle_presentation(workspace, Toggle::Navigation(0), cx),
            Some((EXPLORER.title.into(), contribution_icon(EXPLORER.icon)))
        );
    });
}

#[gpui_kit::test]
fn the_toggle_action_hides_the_shown_explorer_and_shows_it_again(cx: &mut TestAppContext) {
    let (window, workspace) = registered(cx);
    let dispatch = |cx: &mut TestAppContext| {
        cx.update_window(window, |_, window, cx| {
            window.focus(&workspace.read(cx).focus_handle(cx), cx);
            window.dispatch_action(Box::new(ToggleExplorer), cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let open =
        |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).panel_is_open(&EXPLORER.id));
    assert!(open(cx), "the sidebar opens on its first panel");
    dispatch(cx);
    assert!(!open(cx));
    dispatch(cx);
    assert!(open(cx));
}
