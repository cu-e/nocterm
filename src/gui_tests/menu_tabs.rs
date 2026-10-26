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
    ) -> Result<(), nocterm_workspace::LocalDirectoryError> {
        Ok(())
    }
}
fn find_action(items: &[MenuItem], label: &str) -> Option<bool> {
    items.iter().find_map(|item| match item {
        MenuItem::Action { name, disabled, .. } if name.as_ref() == label => Some(!disabled),
        MenuItem::Submenu(menu) => find_action(&menu.items, label),
        _ => None,
    })
}
fn enabled(menus: &[Menu], label: &str) -> bool {
    menus
        .iter()
        .find_map(|menu| find_action(&menu.items, label))
        .unwrap_or_else(|| panic!("application menu action missing: {label}"))
}
fn checked(menus: &[Menu], label: &str) -> bool {
    fn find(items: &[MenuItem], label: &str) -> Option<bool> {
        items.iter().find_map(|item| match item {
            MenuItem::Action { name, checked, .. } if name.as_ref() == label => Some(*checked),
            MenuItem::Submenu(menu) => find(&menu.items, label),
            _ => None,
        })
    }
    menus
        .iter()
        .find_map(|menu| find(&menu.items, label))
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
        assert!(workspace.read(cx).items(cx).next().is_none());
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

#[gpui_kit::test]
fn session_menu_follows_the_terminal_handlers_and_reaches_it_from_the_sidebar(
    cx: &mut TestAppContext,
) {
    use nocterm_session::{CloseReason, Event};
    let (handle, workspace, terminal, transport) = fixture(cx);
    super::emit(cx, &transport, 0, Event::Connected);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let menus = crate::app_menus::build(workspace.read(cx), window, cx);
        assert!(enabled(&menus, "Disconnect Session"));
        assert!(enabled(&menus, "Start Recording"));
        assert!(!enabled(&menus, "Stop Recording"));
        assert!(
            !enabled(&menus, "Find Next"),
            "nothing has been searched yet"
        );
    })
    .unwrap();
    super::emit(
        cx,
        &transport,
        0,
        Event::Closed(CloseReason::Exited(Some(0))),
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let menus = crate::app_menus::build(workspace.read(cx), window, cx);
        assert!(!enabled(&menus, "Disconnect Session"));
        assert!(!enabled(&menus, "Start Recording"));
        assert!(enabled(&menus, "Reconnect Session"));
        workspace.update(cx, |workspace, cx| {
            workspace.open_panel(&nocterm_files::plugin::EXPLORER.id, window, cx)
        });
        window.render_frame(cx);
        assert!(
            !terminal
                .read(cx)
                .focus_handle(cx)
                .contains_focused(window, cx)
        );
        let menus = crate::app_menus::build(workspace.read(cx), window, cx);
        assert!(enabled(&menus, "Find"), "the sidebar forwards to the tab");
        window.dispatch_action(Box::new(nocterm_workspace::Find), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("find-previous").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn window_menu_follows_the_agent_panel_and_the_sidebar_as_the_workspace_reports_them(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, _, _) = fixture(cx);
    let agents = &nocterm_agent::plugin::CHATS.id;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let menus = crate::app_menus::build(workspace.read(cx), window, cx);
        assert!(enabled(&menus, "AI Agents"));
        assert!(!checked(&menus, "AI Agents"));
        assert!(checked(&menus, "Sidebar"), "the sidebar is open at boot");

        window.dispatch_action(Box::new(nocterm_workspace::ToggleSidebar), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let menus = crate::app_menus::build(workspace.read(cx), window, cx);
        assert!(!checked(&menus, "Sidebar"), "toggling hides the sidebar");

        workspace.update(cx, |workspace, cx| {
            workspace.toggle_panel(agents, window, cx)
        });
        window.render_frame(cx);
        let menus = crate::app_menus::build(workspace.read(cx), window, cx);
        assert!(checked(&menus, "AI Agents"));

        workspace.update(cx, |workspace, cx| {
            workspace.set_panel_available(agents, false, window, cx)
        });
        window.render_frame(cx);
        let menus = crate::app_menus::build(workspace.read(cx), window, cx);
        assert!(!enabled(&menus, "AI Agents"));
        assert!(!checked(&menus, "AI Agents"));
    })
    .unwrap();
}
