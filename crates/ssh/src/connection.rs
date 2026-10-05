//! The life of one SSH connection: reach the host, verify it, sign in, run
//! the shell, report how it ended.

use std::{
    borrow::Cow,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use nocterm_session::{
    CloseReason, Command, ConnectRequest, ConnectStage, Event, HostKeyDecision, Prompt, PtySize,
    SessionDriver, SessionError, Target,
};
use russh::{
    Channel, ChannelMsg, Disconnect, Preferred,
    client::{self, DisconnectReason, Handle, Msg},
    keys::{HashAlg, PublicKey, PublicKeyOrCertificate},
};
use tokio::time::timeout;

use crate::{
    SshConfig, auth,
    exec::{self, ExecRequests},
    host_keys::{HostKeys, Verdict, same_kind},
    sftp::{self, FsRequests},
};

/// Unanswered keepalives after which the connection counts as lost.
const KEEPALIVE_MAX: usize = 3;

/// How long the host may take to start the shell once the user is signed in.
const SHELL_TIMEOUT: Duration = Duration::from_secs(30);

/// What the connection's protocol callbacks have seen so far.
#[derive(Default)]
pub(crate) struct Observed {
    host_key: Option<(PublicKey, Verdict)>,
    disconnect: Option<String>,
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
    /// Only a *changed* key ends the handshake here. An unknown one is put to
    /// the user afterwards, before any credential leaves this machine: asking
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
        let acceptable = !matches!(verdict, Verdict::Changed { .. } | Verdict::Revoked { .. });
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

/// Runs a session from the first packet to [`Event::Closed`].
pub(crate) async fn run(
    config: Arc<SshConfig>,
    request: ConnectRequest,
    driver: SessionDriver,
    fs_requests: FsRequests,
    exec_requests: ExecRequests,
) {
    let reason = session(&config, &request, &driver, fs_requests, exec_requests).await;
    tracing::debug!(target = %request.target, ?reason, "session closed");
    driver.emit(Event::Closed(reason)).await;
}

async fn session(
    config: &SshConfig,
    request: &ConnectRequest,
    driver: &SessionDriver,
    fs_requests: FsRequests,
    exec_requests: ExecRequests,
) -> CloseReason {
    let observed = Arc::new(Mutex::new(Observed::default()));
    let mut size = request.size;

    // Until the shell runs, the only commands that matter are a new size to
    // start it with and the user giving up.
    let signed_in = tokio::select! {
        () = driver.closed() => return CloseReason::ClosedByUser,
        result = sign_in(config, request, driver, &observed) => result,
        () = closed_while_connecting(driver, &mut size) => return CloseReason::ClosedByUser,
    };
    let handle = match signed_in {
        Ok(handle) => Arc::new(handle),
        Err(error) => return CloseReason::Failed(error),
    };

    driver
        .emit(Event::Connecting(ConnectStage::StartingShell))
        .await;
    if let Err(error) = request.launch.validate() {
        return CloseReason::Failed(SessionError::Other(error));
    }
    let starting = tokio::select! {
        () = driver.closed() => return CloseReason::ClosedByUser,
        result = timeout(SHELL_TIMEOUT, start_shell(&handle, &request.term, size, &request.launch)) => result,
    };
    let shell = match starting {
        Ok(Ok(shell)) => shell,
        Ok(Err(error)) => return CloseReason::Failed(lost(&observed, &error)),
        Err(_) => {
            return CloseReason::Failed(SessionError::Other(
                "the host did not start a shell".into(),
            ));
        }
    };
    if !driver.emit(Event::Connected).await {
        return CloseReason::ClosedByUser;
    }

    let (shutdown, stopped) = tokio::sync::oneshot::channel();
    let mut files = tokio::spawn(sftp::serve(handle.clone(), fs_requests, stopped));
    let (stop_programs, programs_stopped) = tokio::sync::oneshot::channel();
    let mut programs = tokio::spawn(exec::serve(
        handle.clone(),
        observed.clone(),
        exec_requests,
        programs_stopped,
    ));
    let reason = tokio::select! {
        () = driver.closed() => CloseReason::ClosedByUser,
        reason = run_shell(shell, driver, &observed) => reason,
    };
    let _ = shutdown.send(());
    let _ = stop_programs.send(());
    if tokio::time::timeout(Duration::from_secs(3), &mut files)
        .await
        .is_err()
    {
        files.abort();
    }
    if tokio::time::timeout(Duration::from_secs(1), &mut programs)
        .await
        .is_err()
    {
        programs.abort();
    }

    // Fails only if the connection is already gone.
    let _ = handle.disconnect(Disconnect::ByApplication, "", "en").await;
    reason
}

/// Reaches the host, verifies its key and authenticates the user.
async fn sign_in(
    config: &SshConfig,
    request: &ConnectRequest,
    driver: &SessionDriver,
    observed: &Arc<Mutex<Observed>>,
) -> Result<Handle<Client>, SessionError> {
    let target = &request.target;
    let writable = config.known_hosts.clone();
    let read_only = config.read_only_known_hosts.clone();
    let host_keys = tokio::task::spawn_blocking(move || HostKeys::load(writable, read_only))
        .await
        .map_err(|error| SessionError::Other(format!("cannot load host trust records: {error}")))?
        .map_err(|error| SessionError::Other(error.to_string()))?;

    driver
        .emit(Event::Connecting(ConnectStage::Connecting))
        .await;
    let mut handle = connect(request, &host_keys, observed).await?;
    verify_host(&handle, target, &host_keys, driver, observed).await?;

    driver
        .emit(Event::Connecting(ConnectStage::Authenticating))
        .await;
    auth::authenticate(&mut handle, request, config, driver).await?;
    Ok(handle)
}

async fn connect(
    request: &ConnectRequest,
    host_keys: &HostKeys,
    observed: &Arc<Mutex<Observed>>,
) -> Result<Handle<Client>, SessionError> {
    let Target { host, port, .. } = &request.target;

    // Ask for a kind of key that is already on record, if there is one.
    let known = host_keys.known_algorithms(host, *port);
    let mut preferred = Preferred::default();
    let mut algorithms = preferred.key.to_vec();
    algorithms.sort_by_key(|algorithm| !known.iter().any(|known| same_kind(known, algorithm)));
    preferred.key = Cow::Owned(algorithms);

    let ssh = client::Config {
        keepalive_interval: request.keepalive_interval,
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

    let handshake = async {
        let socket = crate::proxy::connect(&request.target, &request.proxy)
            .await
            .map_err(|error| SessionError::Unreachable {
                host: host.clone(),
                reason: error.to_string(),
            })?;
        // Keystrokes are tiny packets; batching them only adds latency.
        let _ = socket.set_nodelay(true);

        client::connect_stream(Arc::new(ssh), socket, client)
            .await
            .map_err(|error| match lock(observed).host_key.take() {
                Some((_, Verdict::Changed { known_hosts, line })) => SessionError::HostKeyChanged {
                    host: host.clone(),
                    known_hosts,
                    line,
                },
                Some((_, Verdict::Revoked { known_hosts, line })) => {
                    revoked(host, &known_hosts, line)
                }
                _ => SessionError::Other(error.to_string()),
            })
    };

    timeout(request.connect_timeout, handshake)
        .await
        .map_err(|_| SessionError::TimedOut {
            host: host.clone(),
            seconds: request.connect_timeout.as_secs(),
        })?
}

/// Makes sure the user trusts the host before anything secret is sent to it.
async fn verify_host(
    handle: &Handle<Client>,
    target: &Target,
    host_keys: &HostKeys,
    driver: &SessionDriver,
    observed: &Arc<Mutex<Observed>>,
) -> Result<(), SessionError> {
    let host = &target.host;
    let rejected = || SessionError::HostKeyRejected { host: host.clone() };

    let (key, verdict) = lock(observed).host_key.take().ok_or_else(rejected)?;
    match verdict {
        Verdict::Known => return Ok(()),
        Verdict::Changed { known_hosts, line } => {
            return Err(SessionError::HostKeyChanged {
                host: host.clone(),
                known_hosts,
                line,
            });
        }
        Verdict::Revoked { known_hosts, line } => return Err(revoked(host, &known_hosts, line)),
        Verdict::Unknown => {}
    }

    let decision = driver
        .ask(|reply| Prompt::UnknownHostKey {
            host: host_label(target),
            algorithm: key.algorithm().as_str().to_owned(),
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
            reply,
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
        HostKeyDecision::AcceptOnce => Ok(()),
        HostKeyDecision::AcceptAndRemember => {
            if let Err(error) = host_keys.remember(host, target.port, &key) {
                // The user wanted in; a read-only disk should not keep them out.
                tracing::warn!(%host, %error, "could not record the host key");
            }
            Ok(())
        }
    }
}

fn revoked(host: &str, file: &std::path::Path, line: usize) -> SessionError {
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

async fn closed_while_connecting(driver: &SessionDriver, size: &mut PtySize) {
    loop {
        match driver.next_command().await {
            Some(Command::Resize(new_size)) => *size = new_size,
            // Nothing is listening yet; typing ahead of the prompt is dropped.
            Some(Command::Input(_)) => {}
            Some(Command::Close) | None => return,
        }
    }
}

async fn start_shell(
    handle: &Handle<Client>,
    term: &str,
    size: PtySize,
    launch: &nocterm_session::ShellLaunch,
) -> Result<Channel<Msg>, russh::Error> {
    open_terminal(handle, term, size, remote_command(launch)).await
}

/// Opens a channel with a terminal and starts `command` on it, or the
/// user's shell when there is none.
pub(crate) async fn open_terminal(
    handle: &Handle<Client>,
    term: &str,
    size: PtySize,
    command: Option<String>,
) -> Result<Channel<Msg>, russh::Error> {
    let mut channel = handle.channel_open_session().await?;
    channel
        .request_pty(
            true,
            term,
            size.cols.into(),
            size.rows.into(),
            size.pixel_width.into(),
            size.pixel_height.into(),
            &[],
        )
        .await?;
    confirmed(&mut channel).await?;
    match command {
        Some(command) => channel.exec(true, command).await?,
        None => channel.request_shell(true).await?,
    }
    confirmed(&mut channel).await?;
    Ok(channel)
}

/// SSH exec has shell-string semantics. Every configured value is one quoted
/// argument; only the default server shell variable is expanded remotely.
fn remote_command(launch: &nocterm_session::ShellLaunch) -> Option<String> {
    if launch.program.is_none()
        && launch.args.is_empty()
        && launch.cwd.is_none()
        && launch.env.is_empty()
    {
        return None;
    }
    use nocterm_session::quote_posix;
    let mut command = String::new();
    if let Some(cwd) = &launch.cwd {
        command.push_str(&format!("cd -- {} && ", quote_posix(cwd)));
    }
    command.push_str("exec env -- ");
    for (name, value) in &launch.env {
        command.push_str(&quote_posix(&format!("{name}={value}")));
        command.push(' ');
    }
    match &launch.program {
        Some(program) => command.push_str(&quote_posix(program)),
        None => command.push_str("\"${SHELL:-/bin/sh}\""),
    }
    if launch.program.is_none() && launch.args.is_empty() {
        command.push_str(" -l");
    }
    for argument in &launch.args {
        command.push(' ');
        command.push_str(&quote_posix(argument));
    }
    Some(command)
}

/// Waits for the host's answer to the request just made on a channel.
pub(crate) async fn confirmed(channel: &mut Channel<Msg>) -> Result<(), russh::Error> {
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Success) => return Ok(()),
            Some(ChannelMsg::Failure) => return Err(russh::Error::RequestDenied),
            Some(_) => {}
            None => return Err(russh::Error::Disconnect),
        }
    }
}

/// Shuttles bytes between the remote shell and the session's owner.
pub(crate) async fn run_shell(
    channel: Channel<Msg>,
    driver: &SessionDriver,
    observed: &Arc<Mutex<Observed>>,
) -> CloseReason {
    let (mut output, input) = channel.split();
    let mut exit_status = None;

    loop {
        tokio::select! {
            message = output.wait() => match message {
                Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                    if !driver.emit(Event::Output(data.to_vec())).await {
                        return CloseReason::ClosedByUser;
                    }
                }
                Some(ChannelMsg::ExitStatus { exit_status: status }) => exit_status = Some(status),
                Some(ChannelMsg::Close) => return CloseReason::Exited(exit_status),
                Some(_) => {}
                None if exit_status.is_some() => return CloseReason::Exited(exit_status),
                None => return CloseReason::Failed(lost(observed, &"the host closed the connection")),
            },
            command = driver.next_command() => {
                let sent = match command {
                    Some(Command::Input(bytes)) => input.data_bytes(bytes).await,
                    Some(Command::Resize(size)) => {
                        input
                            .window_change(
                                size.cols.into(),
                                size.rows.into(),
                                size.pixel_width.into(),
                                size.pixel_height.into(),
                            )
                            .await
                    }
                    Some(Command::Close) | None => return CloseReason::ClosedByUser,
                };
                if let Err(error) = sent {
                    return CloseReason::Failed(lost(observed, &error));
                }
            }
        }
    }
}

/// The connection broke: prefer the host's own explanation, if it gave one.
fn lost(observed: &Arc<Mutex<Observed>>, fallback: &dyn std::fmt::Display) -> SessionError {
    SessionError::ConnectionLost(
        lock(observed)
            .disconnect
            .take()
            .unwrap_or_else(|| fallback.to_string()),
    )
}

fn lock(observed: &Arc<Mutex<Observed>>) -> MutexGuard<'_, Observed> {
    // The state is plain data, valid even if a holder panicked.
    observed.lock().unwrap_or_else(PoisonError::into_inner)
}
