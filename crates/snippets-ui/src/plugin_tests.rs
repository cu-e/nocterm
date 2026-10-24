//! The snippet library registers under its descriptor and the toggle action drives that id.
use super::{LIBRARY, PLUGIN};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, Focusable as _, TestAppContext, WindowOptions,
};
use nocterm_ui::{Design, DesignTokens, contribution_icon};
use nocterm_workspace::{
    Workspace,
    test_support::{Toggle, deps_for, toggle_presentation},
};

fn registered_workspace(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
    let (handle, workspace, installed) = cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(Design::new(DesignTokens::builtin()));
        let settings = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
        let installed = crate::test_support::install(
            settings,
            &nocterm_store::ShutdownCoordinator::default(),
            cx,
        )
        .unwrap();
        let (handle, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Workspace::new(deps_for(&[&PLUGIN], cx), window, cx))
            })
            .unwrap();
        (handle, workspace, installed)
    });
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            crate::register(workspace, &installed, window, cx).unwrap();
        });
    })
    .unwrap();
    (handle, workspace)
}

#[gpui_kit::test]
fn the_sidebar_toggle_shows_the_descriptors_title_and_icon(cx: &mut TestAppContext) {
    let (_, workspace) = registered_workspace(cx);
    cx.update(|cx| {
        let workspace = workspace.read(cx);
        assert!(workspace.is_registered(&LIBRARY.id));
        assert_eq!(
            toggle_presentation(workspace, Toggle::Navigation(0), cx),
            Some((LIBRARY.title.into(), contribution_icon(LIBRARY.icon)))
        );
    });
}

#[gpui_kit::test]
fn the_toggle_action_hides_the_shown_library_and_shows_it_again(cx: &mut TestAppContext) {
    let (handle, workspace) = registered_workspace(cx);
    let dispatch = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.focus(&workspace.read(cx).focus_handle(cx), cx);
            window.dispatch_action(Box::new(crate::ToggleSnippets), cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let open =
        |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).panel_is_open(&LIBRARY.id));
    assert!(open(cx), "the sidebar opens on its first panel");
    dispatch(cx);
    assert!(!open(cx));
    dispatch(cx);
    assert!(open(cx));
}
