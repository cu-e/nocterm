//! Explicit Linux containment for Cloud tool children; the desktop transport is separate.
mod policy;
mod shell;
#[cfg(test)]
mod tests;
use futures::future::BoxFuture;
use nocterm_session::{
    ConnectRequest, ConnectionOpenOptions, ConnectionSource, ExecError, ExecFuture, ExecOutput,
    ExecRequest, HostExec, Session, ShellLaunch, TerminalRequest, Transport,
};
use policy::ChildPolicy;
pub use policy::{ContainmentError, ContainmentLimits};
use std::{path::PathBuf, sync::Arc};

/// A successful actual namespace probe is required before this capability exists.
/// Compose one owner per isolated account principal. The supervisor still owns
/// its own cgroup limits and private state, outside the mounted workspace.
#[derive(Clone)]
pub struct ContainedLocal {
    policy: Arc<ChildPolicy>,
    source: ConnectionSource,
}
impl ContainedLocal {
    /// Invoke during bounded native startup, before admitting tool work. Dropping
    /// this future drops the existing process/output owner and cancels the probe.
    /// Neither missing tools nor denied namespaces select an uncontained path.
    pub fn prepare(
        workspace: PathBuf,
        supervisor_state: PathBuf,
        limits: ContainmentLimits,
    ) -> BoxFuture<'static, Result<Self, ContainmentError>> {
        Box::pin(async move {
            let policy = Arc::new(ChildPolicy::new(workspace, supervisor_state, limits)?);
            let request = policy.probe()?;
            let output = crate::LocalExec
                .exec(request)
                .await
                .map_err(ContainmentError::Probe)?;
            if !output.exit().await.success() {
                return Err(ContainmentError::Unavailable);
            }
            Ok(Self {
                policy,
                source: ConnectionSource::default(),
            })
        })
    }
    fn scoped(&self, source: ConnectionSource) -> Self {
        Self {
            policy: self.policy.clone(),
            source,
        }
    }
    fn terminal_request(&self, request: TerminalRequest) -> Session {
        let launch = ShellLaunch {
            program: Some(request.program.program),
            args: request.program.args,
            integration: false,
            ..Default::default()
        };
        self.shell(launch, request.term, request.size)
    }
    fn shell(&self, launch: ShellLaunch, term: String, size: nocterm_session::PtySize) -> Session {
        if let Err(error) = self.source.check() {
            return Session::failed(error.into());
        }
        let launch = match self.policy.launch(launch, &term, size) {
            Ok(request) => request,
            Err(error) => return Session::failed(error.connection_failure().into()),
        };
        crate::open_scoped_retained(
            launch,
            true,
            term,
            size,
            self.source.clone(),
            Some(self.policy.clone()),
        )
        .with_exec(Arc::new(self.clone()))
    }
}
impl HostExec for ContainedLocal {
    fn exec(&self, request: ExecRequest) -> ExecFuture<ExecOutput> {
        if let Err(error) = self.source.check() {
            return Box::pin(async move { Err(error.into()) });
        }
        match self.policy.wrap(request) {
            Ok(request) => crate::exec::ScopedLocalExec::new(self.source.clone()).exec(request),
            Err(_) => Box::pin(async { Err(ExecError::Unsupported) }),
        }
    }
    fn terminal(&self, request: TerminalRequest) -> Session {
        self.terminal_request(request)
    }
}
impl Transport for ContainedLocal {
    fn open(&self, request: ConnectRequest) -> Session {
        self.open_with(request, ConnectionOpenOptions::default())
    }
    fn open_with(&self, request: ConnectRequest, options: ConnectionOpenOptions) -> Session {
        self.scoped(options.source().clone())
            .shell(request.launch, request.term, request.size)
    }
}
