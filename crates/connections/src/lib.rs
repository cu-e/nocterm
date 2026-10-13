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
//! Owns: views, profile/recent publication and detected-server presentation/lifecycle.
//! Does not own: profile formats/rules, files, native defaults/IDs/clock, session transport,
//! credentials, remote OS acquisition, DNS, HTTP, flag files or
//! persistence worker lifecycle.
//! Ports used: SettingsHandle, RemoteFs, SnapshotWriter, ShutdownCoordinator,
//! ProfileIds, ConnectionClock, PublicAddressResolver, CountryLocator, FlagRepository, RemoteOsDetector.
//! Ports provided: ConnectionDirectory. Settings section: none.
//! Globals: none. Installed retains exact owners/settings and observer lifetimes.
//! Invariants: profiles publish after physical success; independent recents survive
//! profile failure; derived recent effects require a successful profile projection.
//! Windows/actions use supplied installation; scheduling/publication remain feature migration debt.

mod installed;
mod session_intents;
pub use installed::{Installed, install};
pub use session_intents::{connect, spec_for_profile};
mod directory;
mod editor;
mod facts;
mod geo;
mod menu;
mod model;
mod observation;
pub mod os;
mod panel;
mod prepared;
pub use prepared::{ConnectionWriters, EditorInputs, PreparedConnections};

#[cfg(test)]
mod test_support;

use gpui_kit::{Context, Window, prelude::*};
use nocterm_workspace::Workspace;

pub use editor::{ConnectionEditor, open_editor};
pub use facts::ServerFacts;
pub use menu::NewTabMenu;
pub use model::{Connections, ConnectionsError};
pub use observation::ObservationDeps;
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
        if workspace.items().next().is_none() && !workspace.local_terminal_is_visible(cx) {
            cx.notify();
        }
    })
    .detach();
    // Learn about servers as their sessions come up.
    let probes = installed.clone();
    cx.subscribe(&cx.entity(), move |workspace, _, event, cx| {
        if *event == nocterm_workspace::WorkspaceEvent::ItemsChanged {
            facts::probe_sessions(&probes, workspace, cx);
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
