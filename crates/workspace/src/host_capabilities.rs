//! What a user plugin may observe about this window's workspace and do to its own panels.
//!
//! A dedicated provider over the workspace. Observing is passive: it reports the active context
//! and the active tab's session that already exist for the user's own tabs and never opens,
//! keeps alive or starts anything. It listens only to the workspace's own events.
//!
//! Acting is limited to a plugin's own panel: showing or closing it. The rules that protect what
//! the user is doing (no replacing a panel that shows, nothing while a panel asks for the user,
//! nothing before the user approved placing the plugin) are the window's own, enforced by one
//! transition in the workspace (`plugin_panels::set_open`), never by the caller.
use std::{cell::Cell, rc::Rc};

use gpui_kit::{AnyWindowHandle, App, Entity, Subscription, WeakEntity, Window};
use nocterm_plugin_api::{
    HostError, LocalId,
    capability::{
        panels::{SetOpen, SetPanelOpen},
        workspace::{Kind, WorkspaceKind, WorkspaceState},
    },
};
use nocterm_plugin_model::{ContributionId, PluginId};
use nocterm_plugin_ports::{Apply, Observe, Surface};
use nocterm_project_model::ContextId;

use crate::{
    Workspace,
    workspace::plugin_panels::{self, Refusal},
};

/// What a user plugin sees of one window and may do to its panels.
///
/// The provider never reads the workspace when it is sampled, because the host may sample from
/// inside an update of the workspace (a render or a click handler) and GPUI panics on a read of
/// an entity that is updating. The workspace's own events refresh a small value instead, after
/// the update that caused them has ended.
pub struct ShellCapabilities {
    state: Rc<Cell<Option<WorkspaceState>>>,
    workspace: WeakEntity<Workspace>,
    window: AnyWindowHandle,
    _events: Subscription,
}

impl ShellCapabilities {
    /// Follows `workspace`, which is shown in `window`. Until the first refresh, which runs once
    /// the current update ends, and after the workspace is gone, the answer is `Unavailable`.
    /// Safe to call from inside an update of the workspace.
    pub fn new(workspace: &Entity<Workspace>, window: AnyWindowHandle, cx: &mut App) -> Self {
        let state = Rc::new(Cell::new(None));
        let seeded = state.clone();
        let weak = workspace.downgrade();
        let seeding = weak.clone();
        cx.defer(move |cx| {
            if let Ok(now) = seeding.read_with(cx, state_of) {
                seeded.set(Some(now));
            }
        });
        let followed = state.clone();
        let events = cx.subscribe(workspace, move |workspace, _, cx| {
            followed.set(Some(state_of(workspace.read(cx), cx)));
        });
        Self {
            state,
            workspace: weak,
            window,
            _events: events,
        }
    }

    /// Withdraws the registration of a contribution whose plugin was uninstalled or replaced. A
    /// panel that shows closes without taking focus, and the contribution can register again.
    ///
    /// The application calls it from inside the update of the workspace that follows the plugin
    /// set, so it takes the workspace instead of reaching it through a handle. A contribution
    /// that does not belong to a user plugin is refused with [`WithdrawRefused`] and stays
    /// registered.
    pub fn withdraw(
        workspace: &mut Workspace,
        id: &ContributionId,
        window: &mut Window,
        cx: &mut gpui_kit::Context<Workspace>,
    ) -> Result<(), WithdrawRefused> {
        plugin_panels::withdraw(workspace, id, window, cx).map_err(|_| WithdrawRefused)
    }
}

/// The contribution is not a user plugin's, so the window does not withdraw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("only the contributions of user plugins can be withdrawn")]
pub struct WithdrawRefused;

fn contribution(plugin: &PluginId, local: &LocalId) -> Result<ContributionId, HostError> {
    ContributionId::in_plugin(plugin, local.as_str()).map_err(|_| HostError::Unavailable)
}

impl From<Refusal> for HostError {
    fn from(refusal: Refusal) -> Self {
        match refusal {
            Refusal::Unavailable => Self::Unavailable,
            Refusal::UserIsBeingAsked => Self::Denied,
            Refusal::NotUserContribution => Self::Unavailable,
        }
    }
}

impl Surface for ShellCapabilities {
    fn permit(&self, plugin: &PluginId, local: &LocalId, cx: &App) -> Result<(), HostError> {
        let id = contribution(plugin, local)?;
        self.workspace
            .read_with(cx, |workspace, _| plugin_panels::permit(workspace, &id))
            .map_err(|_| HostError::Unavailable)?
            .map_err(HostError::from)
    }
}

impl Apply<SetPanelOpen> for ShellCapabilities {
    /// Shows or closes the plugin's own panel without moving focus, under the window's rules.
    fn apply(&self, plugin: &PluginId, request: SetOpen, cx: &mut App) -> Result<(), HostError> {
        let id = contribution(plugin, &request.local)?;
        let workspace = self.workspace.clone();
        self.window
            .update(cx, |_, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    plugin_panels::set_open(workspace, &id, request.open, window, cx)
                })
            })
            .map_err(|_| HostError::Unavailable)?
            .map_err(|_| HostError::Unavailable)?
            .map_err(HostError::from)
    }
}

impl Observe<Kind> for ShellCapabilities {
    fn sample(&self, _: &PluginId, _: &App) -> Result<WorkspaceState, HostError> {
        // A window that is gone has nothing to report, whatever it last said. Upgrading takes no
        // lease, so this is also safe while the workspace is updating.
        if self.workspace.upgrade().is_none() {
            return Err(HostError::Unavailable);
        }
        self.state.get().ok_or(HostError::Unavailable)
    }
}

/// A project is a folder, a session tab is remote and carries the session's state, and anything
/// else is the local machine. The shell cannot tell a container, so none is reported.
fn state_of(workspace: &Workspace, cx: &App) -> WorkspaceState {
    if matches!(workspace.selected_context(cx), ContextId::Project(_)) {
        return WorkspaceState {
            kind: WorkspaceKind::Folder,
            connected: true,
        };
    }
    match workspace.active_session(cx) {
        Some(session) => WorkspaceState {
            kind: WorkspaceKind::Remote,
            connected: session.connected,
        },
        None => WorkspaceState {
            kind: WorkspaceKind::Local,
            connected: true,
        },
    }
}

#[cfg(test)]
mod tests;
