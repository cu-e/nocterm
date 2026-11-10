//! Programs on a terminal of their own, beside a session's shell.
//!
//! A log follower or a shell inside a container is a program, not a login:
//! it runs over the connection a session already has, through
//! [`HostExec::terminal`], and is shown like any other session.

use std::{fmt, sync::Arc};

use crate::{ConnectRequest, ExecRequest, HostExec, PtySize, RemoteFs, Session, Transport};

/// A program to start on a terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalRequest {
    /// The program; its input is the terminal, so `stdin` is not used.
    pub program: ExecRequest,
    /// Terminal type announced to the program (its `TERM` variable).
    pub term: String,
    pub size: PtySize,
}

/// Opens sessions that run one program on a host already reached.
///
/// The sessions carry the host's own file system and program runner, so
/// whatever follows the active session sees the same host it came from.
#[derive(Clone)]
pub struct ProgramTransport {
    exec: Arc<dyn HostExec>,
    fs: Option<Arc<dyn RemoteFs>>,
    program: ExecRequest,
    source: Option<crate::ConnectionSource>,
}

impl ProgramTransport {
    pub fn new(
        exec: Arc<dyn HostExec>,
        fs: Option<Arc<dyn RemoteFs>>,
        program: ExecRequest,
    ) -> Self {
        Self {
            exec,
            fs,
            program,
            source: None,
        }
    }
    /// The host owner supplies its original immutable guard beside its native
    /// executor. Opening never acquires a new source from a current selection.
    pub fn new_with_source(
        exec: Arc<dyn HostExec>,
        fs: Option<Arc<dyn RemoteFs>>,
        program: ExecRequest,
        source: crate::ConnectionSource,
    ) -> Self {
        Self {
            exec,
            fs,
            program,
            source: Some(source),
        }
    }
}

impl Transport for ProgramTransport {
    /// Runs the program; the request's address, credentials and launch
    /// options belong to the connection it reuses and are not looked at.
    fn open_with(&self, request: ConnectRequest, options: crate::ConnectionOpenOptions) -> Session {
        let valid = match &self.source {
            Some(source) => options.relay().is_none() && source.same_owner(options.source()),
            None => options.require_unrestricted().is_ok(),
        };
        if !valid {
            return Session::failed(crate::ConnectionSourceError::Unsupported.into());
        }
        if let Err(error) = options.source().check() {
            return Session::failed(error.into());
        }
        self.open(request)
    }
    fn open(&self, request: ConnectRequest) -> Session {
        if let Some(source) = &self.source
            && let Err(error) = source.check()
        {
            return Session::failed(error.into());
        }

        let session = self.exec.terminal(TerminalRequest {
            program: self.program.clone(),
            term: request.term,
            size: request.size,
        });
        session
            .with_fs(self.fs.clone())
            .with_exec(self.exec.clone())
    }
}

impl fmt::Debug for ProgramTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProgramTransport")
            .field("program", &self.program)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::executor::block_on;

    use super::*;
    use crate::{
        Auth, CloseReason, Event, ExecFuture, ExecOutput, ProxyConfig, SessionError, ShellLaunch,
        Target,
    };

    struct NoTerminal;

    impl HostExec for NoTerminal {
        fn exec(&self, _: ExecRequest) -> ExecFuture<ExecOutput> {
            Box::pin(async { Err(crate::ExecError::Unsupported) })
        }
    }

    fn request() -> ConnectRequest {
        ConnectRequest {
            proxy: ProxyConfig::default(),
            launch: ShellLaunch::default(),
            target: Target::new("user", "example.com", 22),
            auth: Auth::Auto,
            term: "xterm-256color".into(),
            size: PtySize::default(),
            connect_timeout: Duration::from_secs(1),
            keepalive_interval: None,
            jumps: Vec::new(),
        }
    }

    #[test]
    fn program_sessions_share_the_host_and_report_an_unsupported_terminal() {
        let exec: Arc<dyn HostExec> = Arc::new(NoTerminal);
        let transport = ProgramTransport::new(exec.clone(), None, ExecRequest::new("top"));
        let session = transport.open(request());
        assert!(Arc::ptr_eq(&session.exec().unwrap(), &exec));
        assert!(session.fs().is_none());
        let event = block_on(session.next_event());
        assert!(matches!(
            event,
            Some(Event::Closed(CloseReason::Failed(SessionError::Other(_))))
        ));
        assert!(block_on(session.next_event()).is_none());
    }
}

#[cfg(test)]
mod source_tests {
    use super::*;
    use crate::{
        Auth, CloseReason, ConnectionOpenOptions, ConnectionSourceError, Event, ProxyConfig,
        SessionError, ShellLaunch, Target,
        testing::{FakeExec, ScriptedConnectionSource},
    };
    use futures::executor::block_on;
    fn request() -> ConnectRequest {
        ConnectRequest {
            target: Target::new("user", "host", 22),
            auth: Auth::Auto,
            launch: ShellLaunch::default(),
            proxy: ProxyConfig::Direct,
            jumps: Vec::new(),
            term: "xterm".into(),
            size: Default::default(),
            connect_timeout: std::time::Duration::from_secs(1),
            keepalive_interval: None,
        }
    }
    #[test]
    fn program_transport_accepts_only_the_exact_original_source_owner() {
        let a = ScriptedConnectionSource::default();
        let a_source = a.source();
        let b = ScriptedConnectionSource::default();
        let b_source = b.source();
        let exec = Arc::new(FakeExec::default());
        let _driver = exec.queue_terminal(a_source.clone());
        let transport = ProgramTransport::new_with_source(
            exec.clone(),
            None,
            ExecRequest::new("top"),
            a_source.clone(),
        );
        let foreign = transport.open_with(request(), ConnectionOpenOptions::new(b_source));
        assert!(matches!(
            block_on(foreign.next_event()),
            Some(Event::Closed(CloseReason::Failed(SessionError::Source(
                ConnectionSourceError::Unsupported
            ))))
        ));
        assert!(exec.terminal_requests().is_empty());
        let original = transport.open_with(request(), ConnectionOpenOptions::new(a_source));
        assert_eq!(exec.terminal_requests().len(), 1);
        assert!(original.input("A"));
        a.revoke();
        assert!(!original.input("foreign"));
        assert!(b.source().check().is_ok());
    }
}
