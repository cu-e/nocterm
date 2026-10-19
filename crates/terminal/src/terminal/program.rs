//! Tabs that run one program on a host already reached, such as a
//! container's log or a shell inside one.

use std::sync::Arc;

use gpui_kit::Context;
use nocterm_session::{Auth, ProgramTransport, ShellLaunch, Target, Transport};
use nocterm_workspace::{ProgramSpec, SessionContext, SessionSpec};

use super::Terminal;

impl Terminal {
    /// Whether this terminal runs a one-shot host command.
    pub fn is_command(&self) -> bool {
        self.lifecycle.is_command()
    }

    /// A tab running `spec`'s program over its host's own connection, or on
    /// this computer when the host has no session.
    pub fn new_program(deps: crate::Installed, spec: ProgramSpec, cx: &mut Context<Self>) -> Self {
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
        let mut terminal = Self::new_kind(deps, spec, session.is_none(), Some(transport), None, cx);
        terminal.input.set_shell_syntax(shell_syntax);
        terminal
    }

    /// The session, as the workspace shows it to panels.
    ///
    /// A program tab shows the host it runs on, which stays reachable after
    /// the program ends unless the program lost its connection.
    pub fn session_context(&self) -> SessionContext {
        self.lifecycle.context()
    }
}
