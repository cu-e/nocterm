//! SSH transport: opens [`Session`]s over SSH, browses the host over SFTP and
//! runs programs on it beside the shell.
//!
//! This is the only crate that knows the SSH protocol exists. It implements
//! [`Transport`] from `nocterm-session`, and everything it has to say to the
//! rest of the application travels as that crate's events and prompts.
//!
//! The transport owns a small Tokio runtime, so callers need none: a session
//! is driven entirely through the runtime-agnostic channels of [`Session`].
//!
//! Owns: SSH authentication, host-key policy, proxy tunnels, jump-host routes, SFTP and remote execution.
//! Does not own: profiles, credential storage, terminal emulation or session views.
//! Ports provided: `Transport`, `RemoteFs`, `HostExec` from `nocterm-session`.
//! Settings section: consumes session SSH options. Globals: none.
//! Invariants: bounded session channels and independent closure interrupt blocked I/O;
//! the proxy carries only the first connection of a route, and each jump host is
//! verified and authenticated before a forward is opened through it.

mod auth;
mod connection;
mod exec;
mod file;
mod host_keys;
mod login;
mod proxy;
mod route;
mod sftp;

use std::{path::PathBuf, sync::Arc};

use nocterm_session::{ConnectRequest, MAX_JUMP_HOSTS, Session, SessionError, Transport};
use tokio::runtime::{Builder, Runtime};

/// Threads serving every SSH connection of the application.
const WORKER_THREADS: usize = 2;

/// Where the transport finds the user's SSH material.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SshConfig {
    /// The known-hosts file nocterm records accepted host keys in.
    pub known_hosts: PathBuf,
    /// Further known-hosts files, consulted but never written, such as
    /// `~/.ssh/known_hosts`.
    pub read_only_known_hosts: Vec<PathBuf>,
    /// Directory of the default private keys (`id_ed25519` and friends).
    pub identity_dir: Option<PathBuf>,
    /// Whether to offer the keys held by the running SSH agent.
    pub use_agent: bool,
}

/// Opens sessions over SSH.
///
/// Sessions run on the transport's threads: dropping the transport ends them
/// all at once, without a [`nocterm_session::Event::Closed`].
pub struct SshTransport {
    // `None` only while the transport is being dropped.
    runtime: Option<Runtime>,
    config: Arc<SshConfig>,
}

impl SshTransport {
    pub fn new(config: SshConfig) -> std::io::Result<Self> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(WORKER_THREADS)
            .thread_name("nocterm-ssh")
            .enable_all()
            .build()?;
        Ok(Self {
            runtime: Some(runtime),
            config: Arc::new(config),
        })
    }
}

impl Transport for SshTransport {
    fn open(&self, request: ConnectRequest) -> Session {
        let Some(runtime) = &self.runtime else {
            return Session::failed(SessionError::Other("the SSH transport has stopped".into()));
        };
        if let Err(error) = request.launch.validate() {
            return Session::failed(SessionError::InvalidLaunch(error));
        }
        if request.jumps.len() > MAX_JUMP_HOSTS {
            return Session::failed(SessionError::TooManyJumpHosts {
                count: request.jumps.len(),
                limit: MAX_JUMP_HOSTS,
            });
        }
        let (fs, fs_requests) = sftp::channel();
        let (exec, exec_requests) = exec::channel(runtime.handle().clone());
        let (session, driver) = nocterm_session::channel(Some(Arc::new(fs)));
        let session = session.with_exec(Arc::new(exec));
        runtime.spawn(connection::run(
            self.config.clone(),
            request,
            driver,
            fs_requests,
            exec_requests,
        ));
        session
    }
}

impl Drop for SshTransport {
    fn drop(&mut self) {
        // A runtime must not block where it is dropped: that may be a thread
        // that is itself running async code.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}
