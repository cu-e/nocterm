//! Interactive agent authentication runs in its own local terminal tab.
use futures::channel::oneshot;
use gpui_kit::{App, WeakEntity, Window};
use nocterm_agent::{TerminalAuthError, TerminalAuthRequest};
use nocterm_session::{CloseReason, ShellLaunch};
use nocterm_workspace::Workspace;
use std::sync::Arc;

pub(crate) fn opener(installed: nocterm_terminal::Installed) -> nocterm_agent::TerminalAuthOpener {
    Arc::new(move |workspace, request, window, cx| open(&installed, workspace, request, window, cx))
}

pub(crate) fn open(
    installed: &nocterm_terminal::Installed,
    workspace: WeakEntity<Workspace>,
    request: TerminalAuthRequest,
    window: &mut Window,
    cx: &mut App,
) -> oneshot::Receiver<Result<(), TerminalAuthError>> {
    let (send, receive) = oneshot::channel();
    let launch = ShellLaunch {
        program: Some(request.program),
        args: request.args,
        cwd: Some(request.cwd.to_string_lossy().into_owned()),
        env: request.env,
        integration: false,
    };
    if let Err(error) = launch.validate() {
        let _ = send.send(Err(error.into()));
        return receive;
    }
    let transport = Arc::new(nocterm_local::IsolatedLocalTransport(launch.clone()));
    let _ = workspace.update(cx, |workspace, cx| {
        let command = nocterm_terminal::LocalCommand::new(
            launch,
            request.title,
            transport,
            move |reason, _| {
                let outcome = match reason {
                    CloseReason::Exited(Some(0)) => Ok(()),
                    CloseReason::ClosedByUser => Err(TerminalAuthError::Canceled),
                    _ => Err(TerminalAuthError::Failed),
                };
                let _ = send.send(outcome);
            },
        );
        nocterm_terminal::open_local_command(installed, workspace, command, window, cx);
    });
    receive
}
