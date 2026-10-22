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
//!
//! Owns: views, editor inputs, session-intent conversion and detected-server image presentation and weak source projection.
//! Does not own: profile/recent publication, committed authority, profile formats/rules, files, native defaults/IDs/clock, session transport,
//! credentials, remote OS acquisition, DNS, HTTP, flag files or
//! persistence worker lifecycle.
//! Ports used: SettingsHandle, RemoteFs, SnapshotWriter, ShutdownCoordinator,
//! ProfileIds, ConnectionClock, FactsService and original terminal capability receipts.
//! Ports provided: ConnectionDirectory and checked original-workspace parent
//! SessionSpec resolution for app composition. Settings section: none.
//! Globals: none. Installed retains exact owners/settings and observer lifetimes.
//! Invariants: profiles publish after physical success; independent recents survive
//! profile failure; derived recent effects require a successful profile projection.
//! Connections aliases the actual service entity; windows/actions use supplied installation.
//! Raw facts/jobs belong to the supplied actual service entity. Original Item
//! capability receipts retain session authority independently of window projection.
//! Workspace recovery compares original profile, destination and ordered resolved
//! route before opening; it never supplies authentication from project metadata.

mod installed;
mod session_intents;
pub use installed::{Installed, install};
pub use session_intents::{connect, resolve_parent_spec, spec_for_profile};
mod country_policy;
mod directory;
mod editor;
mod facts_presentation;
mod geo;
mod menu;
pub mod os;
mod panel;
mod prepared;
mod probe_sources;
pub use prepared::{ConnectionWriters, EditorInputs, PreparedConnections};

#[cfg(test)]
mod test_support;

use gpui_kit::{Context, Window, prelude::*};
use nocterm_workspace::Workspace;

pub use editor::{ConnectionEditor, open_editor};
pub use menu::NewTabMenu;
pub use nocterm_connections_store::ObservationDeps;
pub use nocterm_connections_store::{ConnectionStore as Connections, ConnectionsError};
pub use nocterm_connections_store::{FactsPrepared, FactsService};
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

/// Adds the connections panel, the new-tab menu and the commands to a
/// workspace.
pub fn register(
    installed: &Installed,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    workspace.set_connection_directory(std::rc::Rc::new(directory::Directory::new(
        installed.clone(),
    )));
    let connections = installed.connections();
    cx.observe(connections, |_, _, cx| {
        cx.emit(nocterm_workspace::WorkspaceEvent::ItemsChanged);
        cx.emit(nocterm_workspace::WorkspaceEvent::ConnectionsChanged);
        cx.notify();
    })
    .detach();
    // Facts feed cached panels and the recent servers on the empty screen.
    cx.observe(installed.facts(), |workspace, _, cx| {
        cx.emit(nocterm_workspace::WorkspaceEvent::ConnectionsChanged);
        if workspace.items(cx).next().is_none() && !workspace.local_terminal_is_visible(cx) {
            cx.notify();
        }
    })
    .detach();
    installed.sources().register(cx.entity().downgrade());
    // Learn about servers as their sessions come up.
    let projection = installed.sources().clone();
    let observed_facts = installed.facts().downgrade();
    let observed_connections = installed.connections().downgrade();
    cx.subscribe(&cx.entity(), move |_, _, event, cx| {
        if *event == nocterm_workspace::WorkspaceEvent::ItemsChanged {
            let (projection, facts, connections) = (
                projection.clone(),
                observed_facts.clone(),
                observed_connections.clone(),
            );
            // App::defer escapes the subscriber lease; weak collaborators cannot retain app teardown.
            gpui_kit::App::defer(cx, move |cx| {
                if let (Some(connections), Some(facts)) = (connections.upgrade(), facts.upgrade()) {
                    projection.scan(&connections, &facts, cx);
                }
            });
        }
    })
    .detach();
    let handle = cx.entity().downgrade();

    let panel = cx.new(|cx| ConnectionsPanel::new(installed.clone(), handle.clone(), window, cx));
    workspace.add_panel(panel, cx);

    let menu = cx.new(|cx| NewTabMenu::new(installed.clone(), handle, window, cx));
    workspace.set_new_tab_menu(menu, cx);

    workspace.register_action(|workspace, _: &ToggleServers, window, cx| {
        workspace.toggle_panel_of::<ConnectionsPanel>(window, cx);
    });
    let editors = installed.clone();
    workspace.register_action(move |_, _: &NewConnection, window, cx| {
        open_editor(editors.clone(), None, cx.entity().downgrade(), window, cx);
    });
}
