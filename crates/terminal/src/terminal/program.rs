//! Tabs that run one program on a host already reached, such as a
//! container's log or a shell inside one.

use std::sync::Arc;

use gpui_kit::Context;
use nocterm_session::{
    Auth, CloseReason, ProgramTransport, Session, ShellLaunch, Target, Transport,
};
use nocterm_workspace::{ProgramSpec, SessionContext, SessionSpec};

use super::{Status, Terminal};

impl Terminal {
    /// Whether this terminal runs a one-shot host command.
    pub fn is_command(&self) -> bool {
        self.local_transport.is_some()
    }

    /// A tab running `spec`'s program over its host's own connection, or on
    /// this computer when the host has no session.
    pub fn new_program(spec: ProgramSpec, cx: &mut Context<Self>) -> Self {
        let ProgramSpec {
            title,
            host,
            program,
            shell_syntax,
        } = spec;
        let session = host.session;
        let target = session.as_ref().map_or_else(
            || Target::new("local", "localhost", 22),
            |session| session.target.clone(),
        );
        let fs = session.as_ref().and_then(|session| session.fs.clone());
        let launch = ShellLaunch {
            program: Some(program.program.clone()),
            args: program.args.clone(),
            integration: false,
            ..ShellLaunch::default()
        };
        let transport: Arc<dyn Transport> = Arc::new(ProgramTransport::new(host.exec, fs, program));
        let spec = SessionSpec {
            profile: None,
            options: Default::default(),
            title,
            target,
            auth: Auth::Auto,
            launch: Some(launch),
            credential: None,
        };
        let mut terminal = Self::new_kind(spec, session.is_none(), Some(transport), None, cx);
        terminal.shell_syntax = shell_syntax;
        terminal
    }

    /// The session, as the workspace shows it to panels.
    ///
    /// A program tab shows the host it runs on, which stays reachable after
    /// the program ends unless the program lost its connection.
    pub fn session_context(&self) -> SessionContext {
        let (target, session) = (self.spec.target.clone(), self.session.as_ref());
        let exec = session.and_then(Session::exec);
        if !self.is_command() {
            return SessionContext::new(target, self.fs.clone(), self.is_connected())
                .with_exec(exec);
        }
        let lost = matches!(self.status, Status::Closed(CloseReason::Failed(_)));
        SessionContext::new(target, session.and_then(Session::fs), !lost).with_exec(exec)
    }
}
