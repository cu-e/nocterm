//! A session opened without a tab so the agent key's line can be written through it, and the
//! wait until it is up.
use crate::Installed;
use gpui_kit::{App, EntityId, SharedString, WeakEntity, Window};
use nocterm_connections_model::profiles::Profile;
use nocterm_workspace::{TerminalStatus, Workspace};
use std::time::Duration;

/// How often the session is looked at while it comes up.
pub(super) const POLL: Duration = Duration::from_millis(250);
/// Looks before an unattended sign-in gives up: one minute.
const CONNECT_POLLS: u32 = 240;
/// Looks before a sign-in the user was asked for gives up: three minutes.
const SIGN_IN_POLLS: u32 = 720;

/// Where the wait for the session stands.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Step {
    /// Not up yet; `signing_in` once the user was asked to sign in.
    Waiting {
        signing_in: bool,
    },
    /// The server can be written to.
    Connected,
    Failed(SharedString),
}

pub(super) struct BackgroundSession {
    workspace: WeakEntity<Workspace>,
    item: EntityId,
    /// The session asked the user to sign in and was shown in its tab; it stays open then.
    shown: bool,
    polls: u32,
}

impl BackgroundSession {
    /// Opens `profile` without a tab, signing in with its saved credentials.
    pub(super) fn open(
        installed: &Installed,
        workspace: &WeakEntity<Workspace>,
        profile: &Profile,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Self> {
        let item =
            crate::session_intents::open_background(installed, workspace, profile, window, cx)?;
        Some(Self {
            workspace: workspace.clone(),
            item,
            shown: false,
            polls: 0,
        })
    }

    /// One look at the session; `open` tells whether the server can be written to now. A
    /// session that asks the user to sign in is shown in its tab, once.
    pub(super) fn poll(&mut self, open: bool, window: &mut Window, cx: &mut App) -> Step {
        if open {
            return Step::Connected;
        }
        self.polls += 1;
        match self.status(cx) {
            None | Some(TerminalStatus::Closed) => {
                return Step::Failed(
                    "Could not connect to the server with its saved credentials.".into(),
                );
            }
            Some(TerminalStatus::AwaitingUser | TerminalStatus::AwaitingVault) if !self.shown => {
                self.shown = true;
                let item = self.item;
                let _ = self.workspace.update(cx, |workspace, cx| {
                    workspace.show_background_session(item, window, cx)
                });
            }
            Some(_) => {}
        }
        let limit = if self.shown {
            SIGN_IN_POLLS
        } else {
            CONNECT_POLLS
        };
        if self.polls >= limit {
            Step::Failed("Connecting to the server timed out.".into())
        } else {
            Step::Waiting {
                signing_in: self.shown,
            }
        }
    }

    fn status(&self, cx: &App) -> Option<TerminalStatus> {
        let workspace = self.workspace.upgrade()?;
        let workspace = workspace.read(cx);
        let terminals = workspace
            .context_terminals(workspace.general_surface_source(cx), cx)
            .ok()?;
        let entry = terminals
            .into_iter()
            .find(|entry| entry.item == self.item)?;
        entry.capabilities.state().info(cx).map(|info| info.status)
    }

    /// Closes the session, unless the user was shown it.
    pub(super) fn close(self, window: &mut Window, cx: &mut App) {
        if self.shown {
            return;
        }
        let _ = self.workspace.update(cx, |workspace, cx| {
            workspace.close_background_session(self.item, window, cx)
        });
    }
}
