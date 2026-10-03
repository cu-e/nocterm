//! SSH transport: opens [`Session`]s over SSH and browses the host over SFTP.
//!
//! This is the only crate that knows the SSH protocol exists. It implements
//! [`Transport`] from `nocterm-session`, and everything it has to say to the
//! rest of the application travels as that crate's events and prompts.
//!
//! The transport owns a small Tokio runtime, so callers need none: a session
//! is driven entirely through the runtime-agnostic channels of [`Session`].

mod auth;
mod connection;
mod file;
mod host_keys;
mod proxy;
mod sftp;

use std::{path::PathBuf, sync::Arc};

use nocterm_session::{ConnectRequest, Session, Transport};
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
        let (fs, fs_requests) = sftp::channel();
        let (session, driver) = nocterm_session::channel(Some(Arc::new(fs)));
        if let Some(runtime) = &self.runtime {
            runtime.spawn(connection::run(
                self.config.clone(),
                request,
                driver,
                fs_requests,
            ));
        }
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
