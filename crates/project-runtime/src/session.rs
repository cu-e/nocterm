use nocterm_project_providers::{
    ContainerLocation, ContainerSessionFactory, LocalTransportFactory,
};
use nocterm_session::{
    ConnectRequest, ExecFuture, ExecOutput, ExecRequest, HostExec, RemoteFs, Session, ShellLaunch,
    TerminalRequest, Transport,
};
use std::sync::Arc;

#[derive(Clone, Copy, Default)]
pub struct NativeContainerSessions;
impl ContainerSessionFactory for NativeContainerSessions {
    fn wrap(&self, base: Arc<dyn Transport>, location: ContainerLocation) -> Arc<dyn Transport> {
        Arc::new(ContainerTransport { base, location })
    }
    fn wrap_local(
        &self,
        factory: LocalTransportFactory,
        location: ContainerLocation,
    ) -> Arc<dyn Transport> {
        self.wrap(Arc::new(LocalLaunchTransport { factory }), location)
    }
}
struct LocalLaunchTransport {
    factory: LocalTransportFactory,
}
impl Transport for LocalLaunchTransport {
    fn open(&self, request: ConnectRequest) -> Session {
        (self.factory)(request.launch.clone()).open(request)
    }
}
struct ContainerTransport {
    base: Arc<dyn Transport>,
    location: ContainerLocation,
}
impl Transport for ContainerTransport {
    fn open(&self, mut request: ConnectRequest) -> Session {
        let command = shell(&self.location);
        request.launch = ShellLaunch {
            program: Some(command.program),
            args: command.args,
            integration: false,
            ..Default::default()
        };
        let session = self.base.open(request).with_fs(None);
        match session.exec() {
            Some(base) => {
                let fs: Arc<dyn RemoteFs> = Arc::new(crate::filesystem::ContainerFilesystem::new(
                    base.clone(),
                    self.location.clone(),
                ));
                session
                    .with_fs(Some(fs.clone()))
                    .with_exec(Arc::new(ContainerExec {
                        base,
                        location: self.location.clone(),
                        fs,
                    }))
            }
            None => session,
        }
    }
}
struct ContainerExec {
    base: Arc<dyn HostExec>,
    location: ContainerLocation,
    fs: Arc<dyn RemoteFs>,
}
impl HostExec for ContainerExec {
    fn exec(&self, request: ExecRequest) -> ExecFuture<ExecOutput> {
        self.base.exec(crate::prepare::container_exec(
            &self.location,
            crate::activation::activate_request(request),
            true,
        ))
    }
    fn terminal(&self, mut request: TerminalRequest) -> Session {
        request.program = crate::prepare::container_exec(
            &self.location,
            crate::activation::activate_request(request.program),
            true,
        );
        request.program.args.insert(2, "--tty".into());
        let session = self.base.terminal(request).with_fs(Some(self.fs.clone()));
        session.with_exec(Arc::new(Self {
            base: self.base.clone(),
            location: self.location.clone(),
            fs: self.fs.clone(),
        }))
    }
}
fn shell(location: &ContainerLocation) -> ExecRequest {
    let program = crate::activation::activate_request(ExecRequest::new("sh").args([
        "-c",
        "command -v bash >/dev/null 2>&1 && exec bash || exec sh",
    ]));
    let mut request = crate::prepare::container_exec(location, program, true);
    request.args.insert(2, "--tty".into());
    request
}
