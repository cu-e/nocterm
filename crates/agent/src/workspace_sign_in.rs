//! The [`SignInTerminal`] that runs the host's opener in the panel's workspace.
use crate::TerminalAuthOpener;
use futures::channel::oneshot;
use gpui_kit::{App, WeakEntity, Window};
use nocterm_agent_chat::{TerminalAuthError, TerminalAuthRequest, sign_in::SignInTerminal};
use nocterm_workspace::Workspace;

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
