//! One SSH login over any byte stream: the handshake, the host-key verdict and
//! the user's decision about it. A jump hop and the destination are the same
//! kind of login; only the labels of their prompts differ.

use std::{
    borrow::Cow,
    path::Path,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use nocterm_session::{
    Auth, ConnectRequest, HostKeyDecision, JumpHost, Prompt, PromptHop, SessionDriver,
    SessionError, Target,
};
use russh::{
    Disconnect, Preferred,
    client::{self, DisconnectReason, Handle},
    keys::{HashAlg, PublicKey, PublicKeyOrCertificate},
};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{
    SshConfig,
    host_keys::{HostKeys, RememberError, Verdict, same_kind},
};

/// Unanswered keepalives after which the connection counts as lost.
const KEEPALIVE_MAX: usize = 3;

/// Who is being logged in to, and how: the destination or one jump host.
pub(crate) struct Login<'a> {
    target: &'a Target,
    auth: &'a Auth,
    hop: Option<PromptHop>,
}

impl<'a> Login<'a> {
    pub(crate) fn destination(request: &'a ConnectRequest) -> Self {
        Self {
            target: &request.target,
            auth: &request.auth,
            hop: None,
        }
    }

    pub(crate) fn jump(index: usize, jump: &'a JumpHost) -> Self {
        Self {
            target: &jump.target,
            auth: &jump.auth,
            hop: Some(PromptHop {
                index,
                target: jump.target.clone(),
            }),
        }
    }

    pub(crate) fn target(&self) -> &'a Target {
        self.target
    }

    pub(crate) fn auth(&self) -> &'a Auth {
        self.auth
    }

    /// The hop prompts are labelled with; `None` for the destination.
    pub(crate) fn hop(&self) -> Option<PromptHop> {
        self.hop.clone()
    }
}

/// What the connection's protocol callbacks have seen so far.
#[derive(Default)]
pub(crate) struct Observed {
    host_key: Option<(PublicKey, Verdict)>,
    disconnect: Option<String>,
}

/// The host's own explanation for a lost connection, if it gave one.
pub(crate) fn take_disconnect(observed: &Arc<Mutex<Observed>>) -> Option<String> {
    lock(observed).disconnect.take()
}

/// The protocol callbacks of one connection.
pub(crate) struct Client {
    host_keys: HostKeys,
    host: String,
    port: u16,
    observed: Arc<Mutex<Observed>>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    /// Records what is known about the host's key.
    ///
    /// Revoked keys end the handshake here. Unknown and changed keys are put
    /// to the user afterwards, before any credential leaves this machine: asking
    /// from inside the handshake would stall it for as long as they think.
    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // Host certificates are never requested, so one cannot be verified.
        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            return Ok(false);
        };

        let verdict = self.host_keys.check(&self.host, self.port, key);
        let acceptable = !matches!(verdict, Verdict::Revoked { .. });
        lock(&self.observed).host_key = Some((key.clone(), verdict));
        Ok(acceptable)
    }

    async fn disconnected(
        &mut self,
        reason: DisconnectReason<Self::Error>,
    ) -> Result<(), Self::Error> {
        let message = match reason {
            DisconnectReason::ReceivedDisconnect(info) if info.message.is_empty() => {
                format!("{:?}", info.reason_code)
            }
            DisconnectReason::ReceivedDisconnect(info) => info.message,
            DisconnectReason::Error(error) => error.to_string(),
        };
        lock(&self.observed).disconnect = Some(message);
        Ok(())
    }
}

/// Loads the trust records afresh. Every login reloads them, because accepting a
/// key writes the file but not the snapshot of a login that has already loaded it.
pub(crate) async fn load_host_keys(config: &SshConfig) -> Result<HostKeys, SessionError> {
    let writable = config.known_hosts.clone();
    let read_only = config.read_only_known_hosts.clone();
    tokio::task::spawn_blocking(move || HostKeys::load(writable, read_only))
        .await
        .map_err(|error| SessionError::Other(format!("cannot load host trust records: {error}")))?
        .map_err(|error| SessionError::Other(error.to_string()))
}

/// Runs the SSH handshake over `stream` and leaves the host's key in `observed`.
pub(crate) async fn handshake<S>(
    login: &Login<'_>,
    stream: S,
    host_keys: &HostKeys,
    keepalive_interval: Option<Duration>,
    observed: &Arc<Mutex<Observed>>,
) -> Result<Handle<Client>, SessionError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let Target { host, port, .. } = login.target();

    // Ask for a kind of key that is already on record, if there is one.
    let known = host_keys.known_algorithms(host, *port);
    let mut preferred = Preferred::default();
    let mut algorithms = preferred.key.to_vec();
    algorithms.sort_by_key(|algorithm| !known.iter().any(|known| same_kind(known, algorithm)));
    preferred.key = Cow::Owned(algorithms);

    let ssh = client::Config {
        keepalive_interval,
        keepalive_max: KEEPALIVE_MAX,
        preferred,
        ..client::Config::default()
    };
    let client = Client {
        host_keys: host_keys.clone(),
        host: host.clone(),
        port: *port,
        observed: observed.clone(),
    };

    client::connect_stream(Arc::new(ssh), stream, client)
        .await
        .map_err(|error| match lock(observed).host_key.take() {
            Some((_, Verdict::Revoked { known_hosts, line })) => revoked(host, &known_hosts, line),
            _ => SessionError::Other(error.to_string()),
        })
}

/// Makes sure the user trusts the host before anything secret is sent to it.
pub(crate) async fn verify_host(
    handle: &Handle<Client>,
    login: &Login<'_>,
    host_keys: &HostKeys,
    driver: &SessionDriver,
    observed: &Arc<Mutex<Observed>>,
) -> Result<(), SessionError> {
    let target = login.target();
    let host = &target.host;
    let rejected = || SessionError::HostKeyRejected { host: host.clone() };

    let (key, verdict) = lock(observed).host_key.take().ok_or_else(rejected)?;
    match &verdict {
        Verdict::Known => return Ok(()),
        Verdict::Revoked { known_hosts, line } => return Err(revoked(host, known_hosts, *line)),
        Verdict::Changed { .. } | Verdict::Unknown => {}
    }
    let changed = matches!(verdict, Verdict::Changed { .. });
    let decision = driver
        .ask(|reply| match verdict {
            Verdict::Changed { known_hosts, line } => Prompt::ChangedHostKey {
                host: host.clone(),
                port: target.port,
                algorithm: key.algorithm().as_str().to_owned(),
                old_fingerprints: host_keys.old_fingerprints(host, target.port, &key),
                fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
                known_hosts,
                line,
                replacement_error: host_keys.replacement_error(host, target.port, &key),
                hop: login.hop(),
                reply,
            },
            _ => Prompt::UnknownHostKey {
                host: host_label(target),
                algorithm: key.algorithm().as_str().to_owned(),
                fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
                hop: login.hop(),
                reply,
            },
        })
        .await
        .unwrap_or(HostKeyDecision::Reject);

    match decision {
        HostKeyDecision::Reject => {
            let _ = handle
                .disconnect(Disconnect::HostKeyNotVerifiable, "", "en")
                .await;
            Err(rejected())
        }
        HostKeyDecision::AcceptOnce => {
            host_keys
                .revalidate()
                .map_err(|e| SessionError::Other(e.to_string()))?;
            Ok(())
        }
        HostKeyDecision::AcceptAndRemember if changed => {
            let records = host_keys.clone();
            let host = host.clone();
            let port = target.port;
            tokio::task::spawn_blocking(move || records.replace(&host, port, &key))
                .await
                .map_err(|error| {
                    SessionError::Other(format!("Could not save the new host key: {error}"))
                })?
                .map_err(|error| SessionError::Other(error.to_string()))
        }
        HostKeyDecision::AcceptAndRemember => {
            match host_keys.remember(host, target.port, &key) {
                Ok(()) => Ok(()),
                Err(RememberError::TrustChanged(error)) => Err(SessionError::Other(error)),
                Err(RememberError::Write(error)) => {
                    // Preserve unknown-host behavior for ordinary write errors.
                    tracing::warn!(%host, %error, "could not record the host key");
                    Ok(())
                }
            }
        }
    }
}

fn revoked(host: &str, file: &Path, line: usize) -> SessionError {
    SessionError::Other(format!(
        "the host key of {host} is revoked (line {line} of {}); refusing to connect",
        file.display()
    ))
}

/// The host as known-hosts files spell it.
fn host_label(target: &Target) -> String {
    if target.port == nocterm_session::DEFAULT_PORT {
        target.host.clone()
    } else {
        format!("[{}]:{}", target.host, target.port)
    }
}

fn lock(observed: &Arc<Mutex<Observed>>) -> MutexGuard<'_, Observed> {
    // The state is plain data, valid even if a holder panicked.
    observed.lock().unwrap_or_else(PoisonError::into_inner)
}
