//! The active host's containers in the sidebar.
//! Captured action intents expose their label without mutable host/action authority.
//!
//! The panel lists the containers of whichever machine the active tab is
//! on — this computer or a remote host — grouped by Compose project, with
//! its images, and starts, stops, pauses, restarts and removes them. A
//! container's log and a shell inside it open as tabs of their own. All of
//! it runs through [`nocterm_containers`] over the connection the host
//! already has; the footer's switcher shows how many containers run.
//!
//! Owns: active-host container presentation, captured operation intents and workspace tabs.
//! Local host execution is supplied explicitly at registration and retained by the model.
//! Does not own: container command formats, host execution or connection establishment.
//! Ports used: workspace active-session/program contracts and session `HostExec`.
//! Ports provided: the `containers` system plugin (`plugin::PLUGIN`) with its sidebar panel.
//! Settings section: none. Globals: none.
//! Invariants: host changes invalidate confirmations; completion retains its original context.
mod model;
mod ops;
mod panel;
pub mod plugin;

use gpui_kit::{AppContext as _, Context, Window};
use nocterm_session::HostExec;
use nocterm_workspace::{PanelRegistration, RegistrationError, Workspace};
use std::sync::Arc;

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
///
/// The action is registered even when the workspace refuses the panel.
pub fn register(
    local: Option<Arc<dyn HostExec>>,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Result<(), RegistrationError> {
    workspace.register_action(|workspace, _: &ToggleContainers, window, cx| {
        workspace.toggle_panel(&plugin::LIST.id, window, cx);
    });
    workspace.register_panel(
        PanelRegistration::new(&plugin::LIST, move |workspace, _, cx| {
            let session = workspace.active_session(cx);
            let handle = cx.entity();
            let model =
                cx.new(|cx| model::ContainersModel::new(&handle, session, local.clone(), cx));
            cx.new(|cx| ContainersPanel::new(model, handle.downgrade(), cx))
        }),
        window,
        cx,
    )
}

#[cfg(test)]
mod tests;
