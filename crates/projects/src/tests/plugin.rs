//! The workspaces panel registers under its descriptor and the toggle action drives that id.
use super::{support::fixture, *};
use crate::plugin::WORKSPACES;
use gpui_kit::Focusable as _;
use nocterm_ui::contribution_icon;
use nocterm_workspace::test_support::{Toggle, toggle_presentation};

#[gpui_kit::test]
fn the_sidebar_toggle_shows_the_descriptors_title_and_icon(cx: &mut TestAppContext) {
    let (_, workspace, _) = fixture(cx);
    cx.update(|cx| {
        let workspace = workspace.read(cx);
        assert!(workspace.is_registered(&WORKSPACES.id));
        assert_eq!(
            toggle_presentation(workspace, Toggle::Navigation(0), cx),
            Some((WORKSPACES.title.into(), contribution_icon(WORKSPACES.icon)))
        );
    });
}

#[gpui_kit::test]
fn the_toggle_action_hides_the_shown_panel_and_shows_it_again(cx: &mut TestAppContext) {
    let (window, workspace, _) = fixture(cx);
    let dispatch = |cx: &mut TestAppContext| {
        cx.update_window(window, |_, window, cx| {
            window.focus(&workspace.read(cx).focus_handle(cx), cx);
            window.dispatch_action(Box::new(ToggleProjects), cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let open =
        |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).panel_is_open(&WORKSPACES.id));
    assert!(open(cx), "the fixture opens the panel");
    dispatch(cx);
    assert!(!open(cx));
    dispatch(cx);
    assert!(open(cx));
}
