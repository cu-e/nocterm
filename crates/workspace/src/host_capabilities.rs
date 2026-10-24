//! What a user plugin may observe about this window's workspace.
//!
//! A dedicated provider over the workspace's public queries. It is passive: it reports the active
//! context and the active tab's session that already exist for the user's own tabs and never
//! opens, keeps alive or starts anything. It listens only to the workspace's own events.
use std::{cell::Cell, rc::Rc};

use gpui_kit::{App, Entity, Subscription, WeakEntity};
use nocterm_plugin_api::{
    HostError,
    capability::workspace::{Kind, WorkspaceKind, WorkspaceState},
};
use nocterm_plugin_model::PluginId;
use nocterm_plugin_ports::Observe;
use nocterm_project_model::ContextId;

use crate::Workspace;

/// The workspace observations of one window.
///
/// The provider never reads the workspace when it is sampled, because the host may sample from
/// inside an update of the workspace (a render or a click handler) and GPUI panics on a read of
/// an entity that is updating. The workspace's own events refresh a small value instead, after
/// the update that caused them has ended.
pub struct ShellCapabilities {
    state: Rc<Cell<Option<WorkspaceState>>>,
    workspace: WeakEntity<Workspace>,
    _events: Subscription,
}

impl ShellCapabilities {
    /// Follows `workspace`. Until the first refresh, which runs once the current update ends,
    /// and after the workspace is gone, the answer is `Unavailable`. Safe to call from inside an
    /// update of the workspace.
    pub fn new(workspace: &Entity<Workspace>, cx: &mut App) -> Self {
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
            _events: events,
        }
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
