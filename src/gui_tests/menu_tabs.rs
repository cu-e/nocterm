//! Real application menu capabilities follow visible local and central tab targets.
use super::fixture;
use gpui_kit::{
    App, AppContext as _, Context, EventEmitter, FocusHandle, Focusable, IntoElement, Menu,
    MenuItem, Render, SharedString, TestAppContext, Window, div, prelude::*,
    test::TestWindowExt as _,
};
use nocterm_workspace::{Item, ItemEvent, LocalTerminal};
use std::path::PathBuf;

struct LocalTab(FocusHandle);
impl EventEmitter<ItemEvent> for LocalTab {}
impl Focusable for LocalTab {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.0.clone()
    }
}
impl Render for LocalTab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().track_focus(&self.0)
    }
}
impl Item for LocalTab {
    fn tab_title(&self, _: &App) -> SharedString {
        "Local".into()
    }
}
impl LocalTerminal for LocalTab {
    fn cwd(&self, _: &App) -> Option<PathBuf> {
        Some("/tmp".into())
    }
    fn change_directory(
        &mut self,
        _: PathBuf,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Result<(), String> {
        Ok(())
    }
}
fn enabled(menus: &[Menu], label: &str) -> bool {
    menus
        .iter()
        .flat_map(|menu| &menu.items)
        .find_map(|item| match item {
            MenuItem::Action { name, disabled, .. } if name.as_ref() == label => Some(!disabled),
            _ => None,
        })
        .unwrap_or_else(|| panic!("application menu action missing: {label}"))
}
fn assert_tab_actions(menus: &[Menu], expected: bool) {
    for label in [
        "Close View",
        "Close All Views",
        "Rename Tab",
        "Copy Connection Name",
        "Next Pane",
        "Previous Pane",
        "Next Tab",
        "Previous Tab",
        "Move Tab Left",
        "Move Tab Right",
    ] {
        assert_eq!(enabled(menus, label), expected, "{label}");
    }
}

#[gpui_kit::test]
fn local_only_application_menus_enable_common_tabs_and_disable_hidden_or_outside_targets(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, _, _) = fixture(cx);
    let local = cx
        .update_window(handle, |_, window, cx| {
            let a = cx.new(|cx| LocalTab(cx.focus_handle()));
            let b = cx.new(|cx| LocalTab(cx.focus_handle()));
            workspace.update(cx, |workspace, cx| {
                workspace.close_item(0, window, cx);
                workspace.add_local_terminal(a, window, cx);
                workspace.add_local_terminal(b.clone(), window, cx);
            });
            window.render_frame(cx);
            b
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(workspace.read(cx).items().next().is_none());
        let menus = crate::app_menus::build(workspace.read(cx), window, cx);
        assert_tab_actions(&menus, true);
        assert!(enabled(&menus, "Split View Vertically"));
        let outside = cx.focus_handle();
        window.focus(&outside, cx);
        let menus = crate::app_menus::build(workspace.read(cx), window, cx);
        assert_tab_actions(&menus, false);
        assert!(!enabled(&menus, "Split View Vertically"));
        window.focus(&local.read(cx).focus_handle(cx), cx);
        workspace.update(cx, |workspace, cx| {
            workspace.toggle_local_terminal(window, cx)
        });
        window.render_frame(cx);
        let menus = crate::app_menus::build(workspace.read(cx), window, cx);
        assert_tab_actions(&menus, false);
        assert!(!enabled(&menus, "Split View Vertically"));
    })
    .unwrap();
}
