//! Saved connections, and the ways to open one.
//!
//! - [`Connections`]: the saved profiles (`connections.toml`) and the recently
//!   opened destinations (`recents.toml`), shared by every view below.
//! - [`ConnectionsPanel`]: the sidebar list, filed into folders.
//! - [`NewTabMenu`]: what the tab strip's "+" shows — quick connect, recent
//!   connections, and a way to save a new one.
//! - The editor dialog ([`open_editor`]).
//!
//! Opening a connection asks the workspace for a session
//! ([`Workspace::open_session`]); this crate never names the terminal.

mod directory;
mod editor;
mod facts;
mod geo;
mod menu;
mod model;
pub mod os;
mod panel;
pub mod store;

#[cfg(test)]
mod test_support;

use gpui_kit::{App, Context, Window, prelude::*};
use nocterm_core::Paths;
use nocterm_workspace::Workspace;

pub use editor::{ConnectionEditor, open_editor};
pub use facts::ServerFacts;
pub use menu::NewTabMenu;
pub use model::{Connections, connect, spec_for_profile};
pub use panel::ConnectionsPanel;

gpui_kit::actions!(
    connections,
    [
        /// Open the form for a new saved connection.
        NewConnection,
        /// Show the saved servers in the sidebar, or hide the sidebar if it shows them.
        ToggleServers,
    ]
);

/// Loads the saved connections and what is known about their servers. With
/// no `paths`, nothing is read, written or fetched.
pub fn init(paths: Option<&Paths>, cx: &mut App) {
    menu::init(cx);
    match paths {
        Some(paths) => Connections::load(paths),
        None => Connections::in_memory(),
    }
    .install(cx);
    match paths {
        Some(paths) => ServerFacts::load(paths),
        None => ServerFacts::in_memory(),
    }
    .install(cx);
}

/// Adds the connections panel, the new-tab menu and the commands to a
/// workspace.
pub fn register(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    workspace.set_connection_directory(std::rc::Rc::new(directory::Directory));
    let connections = Connections::global(cx);
    cx.observe(&connections, |_, _, cx| {
        cx.emit(nocterm_workspace::WorkspaceEvent::ItemsChanged);
        cx.emit(nocterm_workspace::WorkspaceEvent::ConnectionsChanged);
        cx.notify();
    })
    .detach();
    // Facts feed cached panels and the recent servers on the empty screen.
    cx.observe(&ServerFacts::global(cx), |workspace, _, cx| {
        cx.emit(nocterm_workspace::WorkspaceEvent::ConnectionsChanged);
        if workspace.items().next().is_none() && !workspace.local_terminal_is_visible(cx) {
            cx.notify();
        }
    })
    .detach();
    // Learn about servers as their sessions come up.
    cx.subscribe(&cx.entity(), |workspace, _, event, cx| {
        if *event == nocterm_workspace::WorkspaceEvent::ItemsChanged {
            facts::probe_sessions(workspace, cx);
        }
    })
    .detach();
    let handle = cx.entity().downgrade();

    let panel = cx.new(|cx| ConnectionsPanel::new(connections.clone(), handle.clone(), window, cx));
    workspace.add_panel(panel, cx);

    let menu = cx.new(|cx| NewTabMenu::new(connections, handle, window, cx));
    workspace.set_new_tab_menu(menu, cx);

    workspace.register_action(|workspace, _: &ToggleServers, window, cx| {
        workspace.toggle_panel_of::<ConnectionsPanel>(window, cx);
    });
    workspace.register_action(|_, _: &NewConnection, window, cx| {
        open_editor(None, cx.entity().downgrade(), window, cx);
    });
}
