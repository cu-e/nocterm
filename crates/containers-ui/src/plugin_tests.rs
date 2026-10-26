//! The containers list registers under its descriptor and the toggle action drives that id.
use super::LIST;
use crate::tests::fixture;
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};
use nocterm_ui::contribution_icon;
use nocterm_workspace::test_support::{Toggle, toggle_presentation};

fn registered(cx: &mut TestAppContext) -> crate::tests::Fixture {
    let fixture = fixture(cx, None);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.workspace.update(cx, |workspace, cx| {
            crate::register(None, workspace, window, cx).unwrap();
        });
    })
    .unwrap();
    cx.run_until_parked();
    fixture
}

#[gpui_kit::test]
fn the_sidebar_toggle_shows_the_descriptors_title_and_icon(cx: &mut TestAppContext) {
    let fixture = registered(cx);
    cx.update(|cx| {
        let workspace = fixture.workspace.read(cx);
        assert!(workspace.is_registered(&LIST.id));
        assert_eq!(
            toggle_presentation(workspace, Toggle::Navigation(0), cx),
            Some((LIST.title.into(), contribution_icon(LIST.icon)))
        );
    });
}

#[gpui_kit::test]
fn the_toggle_action_hides_the_shown_list_and_shows_it_again(cx: &mut TestAppContext) {
    let fixture = registered(cx);
    let dispatch = |cx: &mut TestAppContext| {
        cx.update_window(fixture.window, |_, window, cx| {
            window.focus(&fixture.workspace.read(cx).focus_handle(cx), cx);
            window.dispatch_action(Box::new(crate::ToggleContainers), cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let open = |cx: &mut TestAppContext| {
        cx.update(|cx| fixture.workspace.read(cx).panel_is_open(&LIST.id))
    };
    assert!(open(cx), "the sidebar opens on its first panel");
    dispatch(cx);
    assert!(!open(cx));
    dispatch(cx);
    assert!(open(cx));
}
