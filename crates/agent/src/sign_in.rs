//! How a chat runs an agent's sign-in command in a terminal.
//!
//! Chats request sign-in only through [`SignInTerminal`]. The panel supplies
//! [`WorkspaceSignIn`], which runs the host's opener in its workspace.
use crate::{TerminalAuthError, TerminalAuthOpener, TerminalAuthRequest};
use futures::channel::oneshot;
use gpui_kit::{App, WeakEntity, Window};
use nocterm_workspace::Workspace;

pub(crate) trait SignInTerminal: 'static {
    /// Opens `request`'s sign-in terminal; resolves when the command ends.
    fn open(
        &self,
        request: TerminalAuthRequest,
        window: &mut Window,
        cx: &mut App,
    ) -> oneshot::Receiver<Result<(), TerminalAuthError>>;
}

/// Runs sign-in terminals through the host's opener in one workspace.
pub(crate) struct WorkspaceSignIn {
    opener: TerminalAuthOpener,
    workspace: WeakEntity<Workspace>,
}

impl WorkspaceSignIn {
    pub(crate) fn new(opener: TerminalAuthOpener, workspace: WeakEntity<Workspace>) -> Self {
        Self { opener, workspace }
    }
}

impl SignInTerminal for WorkspaceSignIn {
    fn open(
        &self,
        request: TerminalAuthRequest,
        window: &mut Window,
        cx: &mut App,
    ) -> oneshot::Receiver<Result<(), TerminalAuthError>> {
        (self.opener)(self.workspace.clone(), request, window, cx)
    }
}
