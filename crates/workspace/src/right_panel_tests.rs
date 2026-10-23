//! The right panel opens beside an empty workspace, maximizes and disables.
use super::fixture;
use crate::test_support::TestPanel;
use gpui_kit::{
    AppContext as _, TestAppContext, component::floating::FloatingCards, px,
    test::TestWindowExt as _,
};

#[gpui_kit::test]
fn independent_right_panel_supports_empty_workspace_maximize_and_disable(cx: &mut TestAppContext) {
    let (handle, workspace) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(px(1800.), px(760.)));
        let panel = TestPanel::builder("right-probe")
            .title("AI Agents")
            .text("AI")
            .build(cx);
        workspace.update(cx, |workspace, cx| {
            workspace.set_right_panel(panel.clone(), window, cx);
            workspace.toggle_right_panel(window, cx);
            assert!(!workspace.right_panel_is_open());
            workspace.set_right_panel_available(true, window, cx);
            workspace.toggle_right_panel(window, cx);
            assert!(workspace.right_panel_is_open());
            assert!(panel.read(cx).is_focused(window));
        });
        window.render_frame(cx);
        assert!(window.try_find("right-probe").is_some());
        assert!(window.try_find("empty-new-tab").is_some());
        assert!(window.try_find("open-settings").is_none());
        let toggle = window.find("toggle-right-panel").bounds();
        assert!(toggle.left() > window.find("toggle-local-terminal").bounds().right());
        workspace.update(cx, |workspace, cx| {
            // With navigation hidden, expansion fills the entire body.
            workspace.toggle_sidebar(window, cx);
            workspace.set_right_panel_maximized(true, cx);
        });
        window.render_frame(cx);
        assert!(window.try_find("empty-new-tab").is_none());
        assert!(window.try_find("toggle-right-panel").is_some());
        assert!(panel.read(cx).maximized());
        let inset = FloatingCards::get(cx).map_or(px(0.), |cards| cards.gap * 2. + px(2.));
        let probe = window.find("right-probe").bounds().size.width;
        assert!((probe - (window.viewport_size().width - inset)).abs() < px(2.));
        workspace.update(cx, |workspace, cx| {
            workspace.set_right_panel_available(false, window, cx)
        });
        window.render_frame(cx);
        assert!(window.try_find("toggle-right-panel").is_none());
        assert!(window.try_find("right-probe").is_none());
        assert!(window.try_find("empty-new-tab").is_some());
        assert!(!panel.read(cx).maximized());
        assert!(workspace.read(cx).focus_handle.is_focused(window));
    })
    .unwrap();
}
