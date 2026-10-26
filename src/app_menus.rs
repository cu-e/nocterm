//! Application menus describe commands; feature Items implement their behavior
//! and decide when it is available by registering handlers.

use gpui_kit::{Action, App, Focusable as _, Menu, MenuItem, Window, component::input};
use nocterm_workspace::{Workspace, *};

/// What the menus read of the window: whether the workspace holds focus, which commands it
/// accepts and whether the active tab can be the target of one.
struct MenuState<'a> {
    workspace: &'a Workspace,
    window: &'a Window,
    cx: &'a App,
    in_workspace: bool,
    has_target: bool,
}

impl MenuState<'_> {
    fn enabled(&self, action: &dyn Action) -> bool {
        self.in_workspace
            && self
                .workspace
                .command_available(action, self.window, self.cx)
    }
}

pub(crate) fn build(workspace: &Workspace, window: &Window, cx: &App) -> Vec<Menu> {
    let in_workspace = workspace.focus_handle(cx).contains_focused(window, cx);
    let state = MenuState {
        workspace,
        window,
        cx,
        in_workspace,
        has_target: in_workspace && workspace.command_title(window, cx).is_some(),
    };
    vec![
        session_menu(&state),
        edit_menu(&state),
        search_menu(&state),
        window_menu(&state),
        Menu::new("Help").items([MenuItem::action("About Nocterm", About)]),
    ]
}

fn session_menu(state: &MenuState) -> Menu {
    let MenuState {
        window,
        cx,
        in_workspace,
        has_target,
        ..
    } = *state;
    Menu::new("Session").items([
        MenuItem::action("New Session", NewTab).disabled(!in_workspace),
        MenuItem::action("New Window", NewWindow),
        MenuItem::separator(),
        MenuItem::action("Disconnect Session", DisconnectSession)
            .disabled(!state.enabled(&DisconnectSession)),
        MenuItem::action("Reconnect Session", ReconnectSession)
            .disabled(!state.enabled(&ReconnectSession)),
        MenuItem::submenu(
            Menu::new("Log").items([
                MenuItem::action("Start Recording", StartRecording)
                    .disabled(!state.enabled(&StartRecording)),
                MenuItem::action("Stop Recording", StopRecording)
                    .disabled(!state.enabled(&StopRecording)),
                MenuItem::separator(),
                MenuItem::action("Open Default Logs Directory", LogsDirectory),
            ]),
        ),
        MenuItem::submenu(
            Menu::new("Preferences").items([
                MenuItem::action("Settings", OpenSettings).disabled(!in_workspace),
                MenuItem::action("AI Settings", OpenAiSettings).disabled(!in_workspace),
                MenuItem::action("Session Settings", SessionSettings)
                    .disabled(!state.enabled(&SessionSettings)),
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
    ])
}

fn edit_menu(state: &MenuState) -> Menu {
    // Cut belongs to native editable fields, never to the terminal screen. Its
    // dispatch path also covers dialog inputs outside the Workspace subtree.
    // During initial window construction no dispatch tree exists yet. State is
    // refreshed again from the rendered tree when the user opens a menu.
    let editing = state.window.focused(state.cx).is_some()
        && state.window.is_action_available(&input::Cut, state.cx);
    let has_target = state.has_target;
    Menu::new("Edit").items([
        MenuItem::action("Undo", input::Undo).disabled(!editing),
        MenuItem::action("Redo", input::Redo).disabled(!editing),
        MenuItem::separator(),
        MenuItem::action("Cut", input::Cut).disabled(!editing),
        MenuItem::action("Copy", input::Copy).disabled(!(editing || state.enabled(&input::Copy))),
        MenuItem::action("Paste", input::Paste)
            .disabled(!(editing || state.enabled(&input::Paste))),
        MenuItem::action("Select All", input::SelectAll)
            .disabled(!(editing || state.enabled(&input::SelectAll))),
        MenuItem::action("Clear Selection", ClearSelection)
            .disabled(!state.enabled(&ClearSelection)),
        MenuItem::separator(),
        MenuItem::action("Copy Connection Name", CopyConnectionName).disabled(!has_target),
        MenuItem::action("Rename Tab", RenameTab).disabled(!has_target),
    ])
}

fn search_menu(state: &MenuState) -> Menu {
    Menu::new("Search").items([
        MenuItem::action("Find", Find).disabled(!state.enabled(&Find)),
        MenuItem::action("Find Next", FindNext).disabled(!state.enabled(&FindNext)),
        MenuItem::action("Find Previous", FindPrevious).disabled(!state.enabled(&FindPrevious)),
        MenuItem::action("Find Next Selected Text", FindNextSelection)
            .disabled(!state.enabled(&FindNextSelection)),
    ])
}

/// The Window menu asks the workspace about panels by contribution id and primary group, so
/// it follows whatever the plugin catalog registers.
fn window_menu(state: &MenuState) -> Menu {
    let MenuState {
        workspace,
        window,
        cx,
        in_workspace,
        has_target,
    } = *state;
    let can_split = in_workspace && workspace.can_split_active(window, cx);
    let agents = &nocterm_agent::plugin::CHATS.id;
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
        MenuItem::submenu(layouts_menu(state)),
        MenuItem::submenu(panels_menu(state)),
        MenuItem::separator(),
        MenuItem::action("Sidebar", ToggleSidebar)
            .checked(workspace.primary_group_is_open())
            .disabled(!in_workspace),
        MenuItem::action("Local Terminal", ToggleLocalTerminal)
            .checked(workspace.local_terminal_is_visible(cx))
            .disabled(!in_workspace),
        MenuItem::action("AI Agents", ToggleRightPanel)
            .checked(workspace.panel_is_open(agents))
            .disabled(!in_workspace || !workspace.panel_is_available(agents)),
    ])
}

/// The layouts to choose from, the selected one checked, the commands that move through them,
/// and the way into the editor.
fn layouts_menu(state: &MenuState) -> Menu {
    let in_workspace = state.in_workspace;
    let choices = state.workspace.layouts().into_iter().map(|choice| {
        MenuItem::action(
            choice.name().to_owned(),
            SwitchLayout {
                id: choice.id().as_str().to_owned().into(),
            },
        )
        .checked(choice.is_selected())
        .disabled(!in_workspace)
    });
    Menu::new("Layouts").items(choices.chain([
        MenuItem::separator(),
        MenuItem::action("Next Layout", NextLayout).disabled(!in_workspace),
        MenuItem::action("Previous Layout", PreviousLayout).disabled(!in_workspace),
        MenuItem::action("Switch Layout…", PickLayout).disabled(!in_workspace),
        MenuItem::action("Customize…", OpenLayoutSettings { select: None }).disabled(!in_workspace),
    ]))
}

/// The panels the selected layout places, checked while they show, with a separator between
/// groups. The panels of user plugins stay apart, in a submenu that names each by its plugin.
fn panels_menu(state: &MenuState) -> Menu {
    let mut items = Vec::new();
    let mut user = Vec::new();
    for group in state.workspace.panel_menu() {
        let (mine, theirs): (Vec<_>, Vec<_>) = group
            .into_iter()
            .partition(|panel| panel.user_plugin().is_none());
        user.extend(theirs.into_iter().map(|panel| panel_item(&panel, state)));
        if mine.is_empty() {
            continue;
        }
        if !items.is_empty() {
            items.push(MenuItem::separator());
        }
        items.extend(mine.iter().map(|panel| panel_item(panel, state)));
    }
    if !user.is_empty() {
        if !items.is_empty() {
            items.push(MenuItem::separator());
        }
        items.push(MenuItem::submenu(Menu::new("User plugins").items(user)));
    }
    Menu::new("Panels").items(items)
}

fn panel_item(panel: &nocterm_workspace::PanelEntry, state: &MenuState) -> MenuItem {
    MenuItem::action(
        panel.label(),
        TogglePanel {
            id: panel.id().as_str().to_owned().into(),
        },
    )
    .checked(panel.is_open())
    .disabled(!state.in_workspace || !panel.is_available())
}
