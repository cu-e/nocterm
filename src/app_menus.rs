//! Application menus describe commands; feature Items implement their behavior.

use gpui_kit::{App, Focusable as _, Menu, MenuItem, Window, component::input};
use nocterm_workspace::{ItemCommand, Workspace, *};

#[expect(clippy::too_many_lines, reason = "predates the limit")]
pub(crate) fn build(workspace: &Workspace, window: &Window, cx: &App) -> Vec<Menu> {
    let in_workspace = workspace.focus_handle(cx).contains_focused(window, cx);
    let enabled = |command| in_workspace && workspace.item_command_enabled(command, window, cx);
    // Cut belongs to native editable fields, never to the terminal screen. Its
    // dispatch path also covers dialog inputs outside the Workspace subtree.
    // During initial window construction no dispatch tree exists yet. State is
    // refreshed again from the rendered tree when the user opens a menu.
    let editing = window.focused(cx).is_some() && window.is_action_available(&input::Cut, cx);
    let has_target = in_workspace && workspace.command_title(window, cx).is_some();
    let can_split = in_workspace && workspace.can_split_active(window, cx);

    vec![
        Menu::new("Session").items([
            MenuItem::action("New Session", NewTab).disabled(!in_workspace),
            MenuItem::action("New Window", NewWindow),
            MenuItem::separator(),
            MenuItem::action("Disconnect Session", DisconnectSession)
                .disabled(!enabled(ItemCommand::Disconnect)),
            MenuItem::action("Reconnect Session", ReconnectSession)
                .disabled(!enabled(ItemCommand::Reconnect)),
            MenuItem::submenu(
                Menu::new("Log").items([
                    MenuItem::action("Start Recording", StartRecording)
                        .disabled(!enabled(ItemCommand::StartRecording)),
                    MenuItem::action("Stop Recording", StopRecording)
                        .disabled(!enabled(ItemCommand::StopRecording)),
                    MenuItem::separator(),
                    MenuItem::action("Open Default Logs Directory", LogsDirectory),
                ]),
            ),
            MenuItem::submenu(
                Menu::new("Preferences").items([
                    MenuItem::action("Settings", OpenSettings).disabled(!in_workspace),
                    MenuItem::action("AI Settings", OpenAiSettings).disabled(!in_workspace),
                    MenuItem::action("Session Settings", SessionSettings)
                        .disabled(!enabled(ItemCommand::SessionSettings)),
                    MenuItem::action("Default Session Settings", DefaultSessionSettings)
                        .disabled(!in_workspace),
                    MenuItem::action("Password Vault", OpenVault)
                        .disabled(!in_workspace || !window.is_action_available(&OpenVault, cx)),
                    MenuItem::separator(),
                    MenuItem::action("Profiles Directory", ProfilesDirectory),
                ]),
            ),
            MenuItem::separator(),
            MenuItem::action("Close View", CloseTab).disabled(!has_target),
            MenuItem::action("Close All Views", CloseAllTabs).disabled(!has_target),
            MenuItem::action("Close Window", CloseWindow),
            MenuItem::action("Exit", Quit),
        ]),
        Menu::new("Edit").items([
            MenuItem::action("Undo", input::Undo).disabled(!editing),
            MenuItem::action("Redo", input::Redo).disabled(!editing),
            MenuItem::separator(),
            MenuItem::action("Cut", input::Cut).disabled(!editing),
            MenuItem::action("Copy", input::Copy)
                .disabled(!(editing || enabled(ItemCommand::Copy))),
            MenuItem::action("Paste", input::Paste)
                .disabled(!(editing || enabled(ItemCommand::Paste))),
            MenuItem::action("Select All", input::SelectAll)
                .disabled(!(editing || enabled(ItemCommand::SelectAll))),
            MenuItem::action("Clear Selection", ClearSelection)
                .disabled(!enabled(ItemCommand::ClearSelection)),
            MenuItem::separator(),
            MenuItem::action("Copy Connection Name", CopyConnectionName).disabled(!has_target),
            MenuItem::action("Rename Tab", RenameTab).disabled(!has_target),
        ]),
        Menu::new("Search").items([
            MenuItem::action("Find", Find).disabled(!enabled(ItemCommand::Find)),
            MenuItem::action("Find Next", FindNext).disabled(!enabled(ItemCommand::FindNext)),
            MenuItem::action("Find Previous", FindPrevious)
                .disabled(!enabled(ItemCommand::FindPrevious)),
            MenuItem::action("Find Next Selected Text", FindNextSelection)
                .disabled(!enabled(ItemCommand::FindNextSelection)),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Split View Horizontally", SplitDown).disabled(!can_split),
            MenuItem::action("Split View Vertically", SplitRight).disabled(!can_split),
            MenuItem::action("Split to the Left", SplitLeft).disabled(!can_split),
            MenuItem::action("Split Above", SplitUp).disabled(!can_split),
            MenuItem::separator(),
            MenuItem::action("Next Pane", NextPane).disabled(!has_target),
            MenuItem::action("Previous Pane", PreviousPane).disabled(!has_target),
            MenuItem::action("Next Tab", NextTab).disabled(!has_target),
            MenuItem::action("Previous Tab", PreviousTab).disabled(!has_target),
            MenuItem::action("Move Tab Left", MoveTabLeft).disabled(!has_target),
            MenuItem::action("Move Tab Right", MoveTabRight).disabled(!has_target),
            MenuItem::separator(),
            MenuItem::action("Sidebar", ToggleSidebar)
                .checked(workspace.sidebar_is_open())
                .disabled(!in_workspace),
            MenuItem::action("Local Terminal", ToggleLocalTerminal)
                .checked(workspace.local_terminal_is_visible(cx))
                .disabled(!in_workspace),
            MenuItem::action("AI Agents", ToggleRightPanel)
                .checked(workspace.right_panel_is_open())
                .disabled(!in_workspace || !workspace.right_panel_is_available()),
        ]),
        Menu::new("Help").items([MenuItem::action("About Nocterm", About)]),
    ]
}
