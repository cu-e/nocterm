//! The life of one SSH connection: reach the host, verify it, sign in, run
//! the shell, report how it ended.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use nocterm_session::{
    CloseReason, Command, ConnectRequest, ConnectStage, Event, PtySize, SessionDriver, SessionError,
};
use russh::{
    Channel, ChannelMsg, Disconnect,
    client::{Handle, Msg},
};
use tokio::time::timeout;

use crate::{
    SshConfig,
    exec::{self, ExecRequests},
    login::{Client, Observed, take_disconnect},
    route,
    sftp::{self, FsRequests},
};

/// How long the host may take to start the shell once the user is signed in.
const SHELL_TIMEOUT: Duration = Duration::from_secs(30);

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
    let established = tokio::select! {
        () = driver.closed() => return CloseReason::ClosedByUser,
        result = route::establish(config, request, driver, &observed) => result,
        () = closed_while_connecting(driver, &mut size) => return CloseReason::ClosedByUser,
    };
    let (handle, route) = match established {
        Ok(established) => established,
        Err(error) => return CloseReason::Failed(error),
    };
    let handle = Arc::new(handle);

    let reason = serve(
        &handle,
        request,
        driver,
        &observed,
        size,
        fs_requests,
        exec_requests,
    )
    .await;
    let reason = match reason {
        CloseReason::Failed(error) => CloseReason::Failed(route.attribute(error)),
        other => other,
    };

    // Fails only if the connection is already gone.
    let _ = handle.disconnect(Disconnect::ByApplication, "", "en").await;
    route.close().await;
    reason
}

/// Runs the shell, the file browser and programs on the signed-in destination.
async fn serve(
    handle: &Arc<Handle<Client>>,
    request: &ConnectRequest,
    driver: &SessionDriver,
    observed: &Arc<Mutex<Observed>>,
    size: PtySize,
    fs_requests: FsRequests,
    exec_requests: ExecRequests,
) -> CloseReason {
    driver
        .emit(Event::Connecting(ConnectStage::StartingShell))
        .await;
    let starting = tokio::select! {
        () = driver.closed() => return CloseReason::ClosedByUser,
        result = timeout(SHELL_TIMEOUT, start_shell(handle, &request.term, size, &request.launch)) => result,
    };
    let shell = match starting {
        Ok(Ok(shell)) => shell,
        Ok(Err(error)) => return CloseReason::Failed(lost(observed, &error)),
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
        reason = run_shell(shell, driver, observed) => reason,
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
    reason
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
    SessionError::ConnectionLost(take_disconnect(observed).unwrap_or_else(|| fallback.to_string()))
}
