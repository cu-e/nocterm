//! The sessions behind tabs: which one is active, finding one for a host,
//! and programs started on a host in a tab of their own, such as an editor
//! opened from the Explorer.
use gpui_kit::{App, Context, SharedString, Window};
use nocterm_session::{ShellLaunch, Target};

use super::{Workspace, WorkspaceEvent};
use crate::SessionContext;

impl Workspace {
    /// The latest published session behind the active central tab. Reading the
    /// snapshot is safe even while that Item is rendering or being updated.
    pub fn active_session(&self, _cx: &App) -> Option<SessionContext> {
        self.announced_session.clone()
    }

    /// A connected session for an explicit destination, even when a utility
    /// tab is active. Prefer the active matching tab, then registration order.
    /// Uses only published snapshots and never reads a feature Item.
    pub fn connected_session_for_target(&self, target: &Target) -> Option<SessionContext> {
        let matches = |session: &&SessionContext| session.connected && session.target == *target;
        self.active_item
            .and_then(|ix| self.items.get(ix))
            .and_then(|open| open.session.as_ref())
            .filter(matches)
            .or_else(|| {
                self.items
                    .iter()
                    .filter_map(|open| open.session.as_ref())
                    .find(matches)
            })
            .cloned()
    }

    pub(super) fn announce_session(&mut self, cx: &mut Context<Self>) {
        let session = self
            .active_item
            .and_then(|ix| self.items.get(ix))
            .and_then(|open| open.session.clone());
        let unchanged = match (&session, &self.announced_session) {
            (Some(now), Some(before)) => now.same_as(before),
            (None, None) => true,
            _ => false,
        };
        if !unchanged {
            self.announced_session = session;
            cx.emit(WorkspaceEvent::ActiveSessionChanged);
        }
    }

    /// Opens a new tab running `program` with `args` in `directory` on the
    /// host of a connected session to `target`. It signs in as that session
    /// did; a saved profile's credentials are reused, otherwise the user is
    /// asked again.
    pub fn open_on_host(
        &mut self,
        target: &Target,
        program: String,
        args: Vec<String>,
        directory: Option<String>,
        title: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        // Published snapshots choose the tab, the active one first; only the
        // chosen tabs are asked how they were opened.
        let connected = |open: &&super::OpenItem| {
            open.session
                .as_ref()
                .is_some_and(|session| session.connected && session.target == *target)
        };
        let spec = self
            .active_item
            .and_then(|ix| self.items.get(ix))
            .into_iter()
            .chain(self.items.iter())
            .filter(connected)
            .find_map(|open| open.handle.session_spec(cx))
            .ok_or_else(|| format!("No connected session to {target} is open."))?;
        let env = spec
            .launch
            .as_ref()
            .map(|launch| launch.env.clone())
            .unwrap_or_default();
        let launch = ShellLaunch {
            program: Some(program),
            args,
            cwd: directory,
            env,
            integration: false,
        };
        launch.validate()?;
        let spec = super::SessionSpec {
            title,
            launch: Some(launch),
            ..spec
        };
        self.open_session(spec, window, cx);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
