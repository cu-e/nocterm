//! Windows whose catalog lists the test plugin, with panels registered by contribution.
use crate::{
    PanelRegistration, Workspace,
    test_support::{TEST_PLUGIN, TestPanel, deps_over},
    workspace::tests::fixture_with_deps,
};
use gpui_kit::{AnyWindowHandle, Context, Entity, TestAppContext, Window};
use nocterm_plugin_model::{PanelChrome, SystemContribution};

/// A window whose catalog lists the shell's contributions and the test plugin.
pub(in crate::workspace) fn fixture_with_test_plugin(
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<Workspace>, nocterm_ui::LayoutHandle) {
    fixture_with_deps(cx, |layout, _| deps_over(layout, &[], &[&TEST_PLUGIN]))
}

/// Registers an already built panel for `contribution`, expandable if the contribution draws
/// its own header.
pub(in crate::workspace) fn register_built(
    workspace: &mut Workspace,
    contribution: &SystemContribution,
    panel: &Entity<TestPanel>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let panel = panel.clone();
    let registration = PanelRegistration::new(contribution, move |_, _, _| panel.clone());
    let own_header = contribution
        .panel
        .is_some_and(|traits| traits.chrome == PanelChrome::OwnHeader);
    let registered = if own_header {
        workspace.register_panel(registration.expandable(), window, cx)
    } else {
        workspace.register_panel(registration, window, cx)
    };
    registered.expect("the panel is in the test catalog");
}
