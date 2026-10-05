//! The active host's containers in the sidebar.
//!
//! The panel lists the containers of whichever machine the active tab is
//! on — this computer or a remote host — grouped by Compose project, with
//! its images, and starts, stops, pauses, restarts and removes them. A
//! container's log and a shell inside it open as tabs of their own. All of
//! it runs through [`nocterm_containers`] over the connection the host
//! already has; the footer's switcher shows how many containers run.
mod model;
mod ops;
mod panel;

use gpui_kit::{AppContext as _, Context, Window};
use nocterm_workspace::Workspace;

pub use panel::ContainersPanel;

gpui_kit::actions!(
    containers,
    [
        /// Show the containers of the active host in the sidebar, or hide the
        /// sidebar if it shows them.
        ToggleContainers,
    ]
);

/// Adds the containers panel to the sidebar of `workspace`.
pub fn register(workspace: &mut Workspace, _window: &mut Window, cx: &mut Context<Workspace>) {
    let handle = cx.entity();
    let session = workspace.active_session(cx);
    let local = nocterm_workspace::host::local_exec(cx);
    let model = cx.new(|cx| model::ContainersModel::new(&handle, session, local, cx));
    let panel = cx.new(|cx| ContainersPanel::new(model, handle.downgrade(), cx));
    workspace.add_panel(panel, cx);
    workspace.register_action(|workspace, _: &ToggleContainers, window, cx| {
        workspace.toggle_panel_of::<ContainersPanel>(window, cx);
    });
}

#[cfg(test)]
mod tests;
