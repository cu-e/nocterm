//! SSH transport: opens [`Session`]s over SSH, browses the host over SFTP and
//! runs programs on it beside the shell.
//!
//! This adapter owns the SSH transport protocol. It implements
//! [`Transport`] from `nocterm-session`, and everything it has to say to the
//! rest of the application travels as that crate's events and prompts.
//!
//! The transport owns a small Tokio runtime, so callers need none: a session
//! is driven entirely through the runtime-agnostic channels of [`Session`].
//!
//! Owns: SSH authentication (including keys kept in the vault), entropy and file reads for new vault keys, host-key policy, proxy tunnels and the bridge from a relay's frame channels to a stream, jump-host routes, SFTP and remote execution.
//! Also owns the edit of the user's `authorized_keys` through an open session's SFTP (`authorized_keys`).
//! Also owns the certificate login path: `Auth::VaultCertificate` offers the vault key together with an OpenSSH user certificate (a certificate for another key is never sent); [ADR 0119](../../docs/adr/0119-agent-ssh-ca.md).
//! `SshTransport::open_certificate` signs a runner in with a chat's in-memory key and its certificate, direct and host-pinned, with no fallback ([ADR 0145](../../docs/adr/0145-per-server-ca-principals-and-runner-certificate-sessions.md)).
//! Does not own: profiles, credential storage, the rules of key text (`nocterm-ssh-keys`), terminal emulation or session views.
//! Ports provided: `Transport`, `RemoteFs`, `HostExec` from `nocterm-session`.
//! Ports used: `RelayDialer`, supplied by the application through `SshTransport::with_relay`.
//! Settings section: consumes session SSH options. Globals: none.
//! Invariants: bounded session channels and independent closure interrupt blocked I/O;
//! the proxy or relay carries only the first connection of a route, the host key checked is
//! always the destination's, and each jump host is verified and authenticated before a
//! forward is opened through it.

//! Proof-mode login uses only one device-issued consent and a pinned host proof,
//! direct curve25519-sha256/Ed25519, and no ambient authentication fallback. The
//! device authorizer is released after authentication; live rekey needs no device.
//! Captured source and independent closure guard the actual native stream polls,
//! SFTP/exec workers and retained outputs. Already admitted effects cannot be undone.

mod auth;
pub mod authorized_keys;
mod certificate_login;
pub use certificate_login::{CertificateLogin, CertificateLoginError};
mod connection;
mod connection_relay;
mod exec;
mod file;
mod host_keys;
pub mod keys;
mod login;
pub mod proof;
mod proxy;
mod route;
mod sftp;
mod source_stream;

use std::{path::PathBuf, sync::Arc};

use nocterm_session::{
    ConnectRequest, MAX_JUMP_HOSTS, RelayDialer, Session, SessionError, Transport,
};
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
    relay: Option<Arc<dyn RelayDialer>>,
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
            relay: None,
        })
    }

    /// Lets sessions whose proxy is a hub open their first stream through `relay`.
    ///
    /// Without one, such a session fails to connect. The relay only supplies the byte
    /// stream: host keys and sign-in are still checked here, against the destination.
    #[must_use]
    pub fn with_relay(mut self, relay: Arc<dyn RelayDialer>) -> Self {
        self.relay = Some(relay);
        self
    }
    pub fn open_login(
        &self,
        request: ConnectRequest,
        options: nocterm_session::ConnectionOpenOptions,
        attempt: proof::SessionLoginAttempt,
    ) -> Session {
        let login = proof::PurposeLogin::Proof(attempt);
        if let Err(error) = login.check(&request) {
            return Session::failed(error);
        }
        self.start(request, options, Some(login))
    }
    /// Opens a session signed in with a chat's key and certificate (ADR 0145): direct,
    /// against the pinned host key, with no other authentication method.
    pub fn open_certificate(
        &self,
        request: ConnectRequest,
        options: nocterm_session::ConnectionOpenOptions,
        login: CertificateLogin,
    ) -> Session {
        let login = proof::PurposeLogin::Certificate(login);
        if let Err(error) = login.check(&request) {
            return Session::failed(error);
        }
        self.start(request, options, Some(login))
    }
    fn start(
        &self,
        request: ConnectRequest,
        options: nocterm_session::ConnectionOpenOptions,
        login: Option<proof::PurposeLogin>,
    ) -> Session {
        if let Err(error) = options.source().check() {
            return Session::failed(error.into());
        }
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
        let relay = match connection_relay::ConnectionRelay::capture(
            &request,
            &options,
            self.relay.clone(),
        ) {
            Ok(captured) => captured.into_port(),
            Err(error) => return Session::failed(error),
        };
        let source = options.source().clone();
        let (fs, fs_requests) = sftp::channel(source.clone());
        let (exec, exec_requests) = exec::channel(runtime.handle().clone(), source.clone());
        let (session, driver) = nocterm_session::channel_with_source(Some(Arc::new(fs)), source);
        let session = session.with_exec(Arc::new(exec));
        runtime.spawn(connection::run(
            self.config.clone(),
            relay,
            request,
            driver,
            fs_requests,
            exec_requests,
            login,
        ));
        session
    }
}

impl Transport for SshTransport {
    fn open(&self, request: ConnectRequest) -> Session {
        self.open_with(request, nocterm_session::ConnectionOpenOptions::default())
    }
    fn open_with(
        &self,
        request: ConnectRequest,
        options: nocterm_session::ConnectionOpenOptions,
    ) -> Session {
        self.start(request, options, None)
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
