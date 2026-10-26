//! A registered panel is framed as its descriptor says: a titled panel sits under the
//! workspace's title and a panel that draws its own header fills its column.
use gpui_kit::{
    AnyWindowHandle, AppContext as _, ElementId, Entity, TestAppContext, px, size,
    test::TestWindowExt as _,
};
use nocterm_plugin_model::SystemContribution;

use super::PanelRegistration;
use crate::{
    Workspace,
    test_support::{
        PANEL_A, REQUIRED_EXPANDABLE, REQUIRED_PLUGIN, TEST_PLUGIN, TestPanel, deps_for,
    },
    workspace::tests::fixture_with_deps,
};

/// The top of the panel's view and of the column that holds it, once the panel shows.
fn tops(
    cx: &mut TestAppContext,
    contribution: &'static SystemContribution,
    column: ElementId,
) -> (gpui_kit::Pixels, gpui_kit::Pixels) {
    let (handle, workspace, _): (AnyWindowHandle, Entity<Workspace>, _) =
        fixture_with_deps(cx, |_, cx| deps_for(&[&TEST_PLUGIN, &REQUIRED_PLUGIN], cx));
    cx.update_window(handle, |_, window, cx| {
        window.resize(size(px(1400.), px(700.)));
        workspace.update(cx, |workspace, cx| {
            let registration = PanelRegistration::new(contribution, |_, _, cx| {
                TestPanel::builder("panel").build(cx)
            });
            workspace
                .register_panel(registration, window, cx)
                .expect("the panel is in the catalog");
            workspace.open_panel(&contribution.id, window, cx);
        });
        window.render_frame(cx);
        window.render_frame(cx);
        let panel = window.find("panel").bounds().top();
        (window.find(column).bounds().top(), panel)
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_titled_panel_sits_under_the_workspaces_title(cx: &mut TestAppContext) {
    let column = ElementId::from((ElementId::from("panel-group-column"), "navigation"));
    let (column, panel) = tops(cx, PANEL_A, column);
    assert!(panel > column, "the title takes the top of the column");
}

#[gpui_kit::test]
fn a_panel_with_its_own_header_is_not_given_a_title(cx: &mut TestAppContext) {
    // Not registered as expandable: the descriptor alone says the panel draws its own header.
    let (column, panel) = tops(
        cx,
        REQUIRED_EXPANDABLE,
        ElementId::from((ElementId::from("panel-group-column"), "assistant")),
    );
    assert_eq!(panel, column, "the panel fills its column");
}
