//! Programs run beside the shell, each on a session channel of its own.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use futures::channel::oneshot;
use nocterm_session::{
    CloseReason, ErrorTail, Event, ExecError, ExecFuture, ExecOutput, ExecRequest, ExecSink,
    HostExec, Session, SessionDriver, SessionError, TerminalRequest,
};
use russh::{
    Channel, ChannelMsg, Sig,
    client::{Handle, Msg},
};
use tokio::{runtime, sync::mpsc, sync::watch, task::JoinSet, time::timeout};

use crate::connection::{Client, Observed, confirmed, open_terminal, run_shell};

const REQUEST_BACKLOG: usize = 16;
/// Programs one connection runs at once; further requests wait their turn.
/// Programs on a terminal are the user's and are not counted.
const CONCURRENT_PROGRAMS: usize = 8;
/// How long the host may take to start a program.
const START_TIMEOUT: Duration = Duration::from_secs(15);
/// The extended-data stream SSH carries standard error on.
const STDERR: u32 = 1;

enum Job {
    Exec {
        request: ExecRequest,
        answer: oneshot::Sender<Result<ExecOutput, ExecError>>,
    },
    Terminal {
        request: TerminalRequest,
        driver: SessionDriver,
    },
}

/// Runs programs on the host of one SSH session.
#[derive(Clone)]
pub(crate) struct SshExec {
    requests: mpsc::Sender<Job>,
    runtime: runtime::Handle,
}

/// The connection's end of [`SshExec`].
pub(crate) struct ExecRequests {
    requests: mpsc::Receiver<Job>,
}

pub(crate) fn channel(runtime: runtime::Handle) -> (SshExec, ExecRequests) {
    let (requests, receiver) = mpsc::channel(REQUEST_BACKLOG);
    (
        SshExec { requests, runtime },
        ExecRequests { requests: receiver },
    )
}

impl HostExec for SshExec {
    fn exec(&self, request: ExecRequest) -> ExecFuture<ExecOutput> {
        let requests = self.requests.clone();
        Box::pin(async move {
            let (answer, answered) = oneshot::channel();
            requests
                .send(Job::Exec { request, answer })
                .await
                .map_err(|_| ExecError::Disconnected)?;
            answered.await.map_err(|_| ExecError::Disconnected)?
        })
    }

    fn terminal(&self, request: TerminalRequest) -> Session {
        let (session, driver) = nocterm_session::channel(None);
        let requests = self.requests.clone();
        self.runtime.spawn(async move {
            let job = Job::Terminal {
                request,
                driver: driver.clone(),
            };
            if requests.send(job).await.is_err() {
                driver.emit(Event::Closed(disconnected())).await;
            }
        });
        session
    }
}

fn disconnected() -> CloseReason {
    CloseReason::Failed(SessionError::ConnectionLost(
        "the session's connection closed".into(),
    ))
}

/// Starts requested programs until shutdown. Programs still running then are
/// dropped with their channels, which the host sees as a hang-up; programs
/// on a terminal are told why first.
pub(crate) async fn serve(
    handle: Arc<Handle<Client>>,
    observed: Arc<Mutex<Observed>>,
    mut requests: ExecRequests,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let mut running = JoinSet::new();
    let mut terminals = JoinSet::new();
    let (stopping, stopped) = watch::channel(false);
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => break,
            Some(_) = running.join_next(), if !running.is_empty() => {}
            Some(_) = terminals.join_next(), if !terminals.is_empty() => {}
            job = requests.requests.recv(), if running.len() < CONCURRENT_PROGRAMS => {
                match job {
                    None => break,
                    Some(Job::Exec { request, answer }) => {
                        running.spawn(run(handle.clone(), request, answer));
                    }
                    Some(Job::Terminal { request, driver }) => {
                        terminals.spawn(terminal(
                            handle.clone(),
                            observed.clone(),
                            request,
                            driver,
                            stopped.clone(),
                        ));
                    }
                }
            }
        }
    }
    running.abort_all();
    let _ = stopping.send(true);
    while running.join_next().await.is_some() {}
    while terminals.join_next().await.is_some() {}
}

async fn run(
    handle: Arc<Handle<Client>>,
    request: ExecRequest,
    answer: oneshot::Sender<Result<ExecOutput, ExecError>>,
) {
    let channel = match timeout(START_TIMEOUT, start(&handle, &request)).await {
        Ok(Ok(channel)) => channel,
        Ok(Err(error)) => {
            let _ = answer.send(Err(ExecError::Failed(error.to_string())));
            return;
        }
        Err(_) => {
            let _ = answer.send(Err(ExecError::Failed(
                "the host did not start the program".into(),
            )));
            return;
        }
    };
    let (sink, output) = ExecOutput::channel();
    if answer.send(Ok(output)).is_ok() {
        forward(channel, sink).await;
    } else {
        let _ = channel.close().await;
    }
}

async fn start(
    handle: &Handle<Client>,
    request: &ExecRequest,
) -> Result<Channel<Msg>, russh::Error> {
    let mut channel = handle.channel_open_session().await?;
    channel.exec(true, request.command_line()).await?;
    confirmed(&mut channel).await?;
    if let Some(stdin) = &request.stdin {
        channel.data(&stdin[..]).await?;
    }
    channel.eof().await?;
    Ok(channel)
}

/// Delivers the program's output until it ends or its reader goes away.
async fn forward(mut channel: Channel<Msg>, sink: ExecSink) {
    let mut errors = ErrorTail::default();
    let mut status = None;
    loop {
        tokio::select! {
            () = sink.closed() => break,
            message = channel.wait() => match message {
                Some(ChannelMsg::Data { data }) => {
                    if !sink.send(data.to_vec()).await {
                        break;
                    }
                }
                Some(ChannelMsg::ExtendedData { data, ext: STDERR }) => errors.push(&data),
                Some(ChannelMsg::ExitStatus { exit_status }) => status = Some(exit_status),
                // The exit status follows the end of output.
                Some(ChannelMsg::Close) | None => {
                    sink.finish(errors.exit(status));
                    return;
                }
                Some(_) => {}
            },
        }
    }
    // Not every server honours signals; closing the channel also breaks the
    // program's output pipe.
    let _ = channel.signal(Sig::TERM).await;
    let _ = channel.close().await;
}

/// Runs a program on a terminal of its own until it ends, its session is
/// closed or the connection goes.
async fn terminal(
    handle: Arc<Handle<Client>>,
    observed: Arc<Mutex<Observed>>,
    request: TerminalRequest,
    driver: SessionDriver,
    mut stopping: watch::Receiver<bool>,
) {
    let reason = tokio::select! {
        () = driver.closed() => CloseReason::ClosedByUser,
        _ = stopping.wait_for(|stopping| *stopping) => disconnected(),
        reason = run_terminal(&handle, &observed, &request, &driver) => reason,
    };
    driver.emit(Event::Closed(reason)).await;
}

async fn run_terminal(
    handle: &Handle<Client>,
    observed: &Arc<Mutex<Observed>>,
    request: &TerminalRequest,
    driver: &SessionDriver,
) -> CloseReason {
    let command = Some(request.program.command_line());
    let channel = match timeout(
        START_TIMEOUT,
        open_terminal(handle, &request.term, request.size, command),
    )
    .await
    {
        Ok(Ok(channel)) => channel,
        Ok(Err(error)) => return CloseReason::Failed(SessionError::Other(error.to_string())),
        Err(_) => {
            return CloseReason::Failed(SessionError::Other(
                "the host did not start the program".into(),
            ));
        }
    };
    if !driver.emit(Event::Connected).await {
        return CloseReason::ClosedByUser;
    }
    run_shell(channel, driver, observed).await
}
