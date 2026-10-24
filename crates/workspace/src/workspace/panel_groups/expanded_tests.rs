//! An expandable side panel opens beside an empty workspace, fills the body and is withdrawn.
use crate::{
    PanelRegistration,
    test_support::{
        EXPANDABLE, TEST_CORE, TEST_PLUGIN, TestPanel, Toggle, deps_for_core, toggle_id,
    },
    workspace::tests::fixture_with_deps,
};
use gpui_kit::{
    AppContext as _, TestAppContext, component::floating::FloatingCards, px, size,
    test::TestWindowExt as _,
};
use nocterm_plugin_model::ContributionId;

#[gpui_kit::test]
fn an_expandable_panel_opens_beside_an_empty_workspace_fills_the_body_and_withdraws(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, _) =
        fixture_with_deps(cx, |_, cx| deps_for_core(&[TEST_CORE], &[&TEST_PLUGIN], cx));
    let id = ContributionId::new(EXPANDABLE.id.as_str());
    cx.update_window(handle, |_, window, cx| {
        window.resize(size(px(1800.), px(760.)));
        let registration = PanelRegistration::new(EXPANDABLE, |_, _, cx| {
            TestPanel::builder("right-probe")
                .title("AI Agents")
                .text("AI")
                .build(cx)
        })
        .expandable();
        workspace.update(cx, |workspace, cx| {
            workspace
                .register_panel(registration, window, cx)
                .expect("the panel is in the catalog");
            workspace.toggle_panel(&id, window, cx);
            assert!(workspace.panel_is_open(&id));
        });
        window.render_frame(cx);
        assert!(window.try_find("right-probe").is_some());
        assert!(window.try_find("empty-new-tab").is_some());
        assert!(window.try_find("open-settings").is_none());
        let toggle = window.find(toggle_id(Toggle::SidePanel)).bounds();
        let terminal = window.find(toggle_id(Toggle::LocalTerminal)).bounds();
        assert!(toggle.left() > terminal.right());
        workspace.update(cx, |workspace, cx| {
            // With navigation hidden, expansion fills the entire body.
            workspace.toggle_sidebar(window, cx);
            workspace.set_panel_expanded(&id, true, cx);
        });
        window.render_frame(cx);
        assert!(window.try_find("empty-new-tab").is_none());
        assert!(window.try_find(toggle_id(Toggle::SidePanel)).is_some());
        assert!(workspace.read(cx).panel_is_expanded(&id));
        let inset = FloatingCards::get(cx).map_or(px(0.), |cards| cards.gap * 2. + px(2.));
        let probe = window.find("right-probe").bounds().size.width;
        assert!((probe - (window.viewport_size().width - inset)).abs() < px(2.));
        workspace.update(cx, |workspace, cx| {
            workspace.set_panel_available(&id, false, window, cx)
        });
        window.render_frame(cx);
        assert!(window.try_find(toggle_id(Toggle::SidePanel)).is_none());
        assert!(window.try_find("right-probe").is_none());
        assert!(window.try_find("empty-new-tab").is_some());
        assert!(!workspace.read(cx).panel_is_expanded(&id));
        assert!(workspace.read(cx).focus_handle.is_focused(window));
    })
    .unwrap();
}
