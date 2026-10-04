//! Interactive agent authentication runs in its own local terminal tab.
use futures::channel::oneshot;
use gpui_kit::{App, WeakEntity, Window};
use nocterm_agent::TerminalAuthRequest;
use nocterm_session::{CloseReason, ShellLaunch};
use nocterm_workspace::Workspace;
use std::sync::Arc;

pub(crate) fn open(
    workspace: WeakEntity<Workspace>,
    request: TerminalAuthRequest,
    window: &mut Window,
    cx: &mut App,
) -> oneshot::Receiver<Result<(), String>> {
    let (send, receive) = oneshot::channel();
    let launch = ShellLaunch {
        program: Some(request.program),
        args: request.args,
        cwd: Some(request.cwd.to_string_lossy().into_owned()),
        env: request.env,
        integration: false,
    };
    if let Err(error) = launch.validate() {
        let _ = send.send(Err(error));
        return receive;
    }
    let transport = Arc::new(nocterm_local::IsolatedLocalTransport(launch.clone()));
    let _ = workspace.update(cx, |workspace, cx| {
        nocterm_terminal::open_local_command(
            workspace,
            launch,
            request.title,
            transport,
            move |reason, _| {
                let outcome = match reason {
                    CloseReason::Exited(Some(0)) => Ok(()),
                    CloseReason::ClosedByUser => Err("Authentication canceled.".into()),
                    _ => Err("Authentication command did not complete successfully.".into()),
                };
                let _ = send.send(outcome);
            },
            window,
            cx,
        );
    });
    receive
}
