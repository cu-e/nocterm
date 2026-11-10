//! Fixed workspace environment exports preserve original command arguments.
use nocterm_project_providers::{LocalTransportFactory, WorkspaceSessionFactory};
use nocterm_session::{
    ConnectRequest, ExecFuture, ExecOutput, ExecRequest, HostExec, Session, ShellLaunch,
    TerminalRequest, Transport,
};
use std::sync::Arc;

pub(crate) fn activate_request(request: ExecRequest) -> ExecRequest {
    let mut activated = ExecRequest::new("sh")
        .args([
            "-c",
            &format!("{}\nexec \"$@\"", crate::managed::shell()),
            "nocterm-workspace",
        ])
        .arg(request.program)
        .args(request.args);
    activated.stdin = request.stdin;
    activated
}

fn activate_launch(mut launch: ShellLaunch) -> ShellLaunch {
    let script = format!(
        "{}\nif [ -n \"$1\" ]; then exec \"$@\"; else shift; exec \"${{SHELL:-/bin/sh}}\" \"$@\"; fi",
        crate::managed::shell()
    );
    let mut args = vec!["-c".into(), script, "nocterm-workspace".into()];
    args.push(launch.program.take().unwrap_or_default());
    args.append(&mut launch.args);
    launch.program = Some("sh".into());
    launch.args = args;
    launch.integration = false;
    launch
}

#[derive(Clone, Copy, Default)]
pub struct NativeWorkspaceSessions;
impl WorkspaceSessionFactory for NativeWorkspaceSessions {
    fn wrap(&self, base: Arc<dyn Transport>) -> Arc<dyn Transport> {
        Arc::new(WorkspaceTransport {
            base: LaunchBase::Remote(base),
        })
    }
    fn wrap_local(&self, factory: LocalTransportFactory) -> Arc<dyn Transport> {
        Arc::new(WorkspaceTransport {
            base: LaunchBase::Local(factory),
        })
    }
}

enum LaunchBase {
    Remote(Arc<dyn Transport>),
    Local(LocalTransportFactory),
}
struct WorkspaceTransport {
    base: LaunchBase,
}
impl Transport for WorkspaceTransport {
    fn open(&self, request: ConnectRequest) -> Session {
        self.open_with(request, Default::default())
    }
    fn open_with(
        &self,
        mut request: ConnectRequest,
        options: nocterm_session::ConnectionOpenOptions,
    ) -> Session {
        let session = match &self.base {
            LaunchBase::Remote(base) => {
                request.launch = activate_launch(request.launch);
                base.open_with(request, options)
            }
            LaunchBase::Local(factory) => {
                factory(request.launch.clone(), Some(Arc::new(LocalActivation)))
                    .open_with(request, options)
            }
        };
        match session.exec() {
            Some(base) => session.with_exec(Arc::new(WorkspaceExec { base })),
            None => session,
        }
    }
}

struct LocalActivation;
impl nocterm_session::ShellLaunchPreparation for LocalActivation {
    fn prepare(
        &self,
        mut launch: ShellLaunch,
    ) -> Result<ShellLaunch, nocterm_session::LaunchValidationError> {
        crate::managed::local(&mut launch);
        launch.validate()?;
        Ok(launch)
    }
}
struct WorkspaceExec {
    base: Arc<dyn HostExec>,
}
impl HostExec for WorkspaceExec {
    fn exec(&self, request: ExecRequest) -> ExecFuture<ExecOutput> {
        self.base.exec(activate_request(request))
    }
    fn terminal(&self, mut request: TerminalRequest) -> Session {
        request.program = activate_request(request.program);
        let session = self.base.terminal(request);
        match session.exec() {
            Some(base) => session.with_exec(Arc::new(Self { base })),
            None => session,
        }
    }
}

#[cfg(test)]
mod tests;

impl nocterm_project_providers::WorkspaceCommandActivation for NativeWorkspaceSessions {
    fn activate_request(&self, request: ExecRequest) -> ExecRequest {
        activate_request(request)
    }
}
