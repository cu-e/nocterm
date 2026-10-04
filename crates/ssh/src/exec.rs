//! Programs run beside the shell, each on a session channel of its own.

use std::{sync::Arc, time::Duration};

use futures::channel::oneshot;
use nocterm_session::{ExecError, ExecFuture, ExecOutput, ExecRequest, ExecSink, HostExec};
use russh::{
    Channel, ChannelMsg, Sig,
    client::{Handle, Msg},
};
use tokio::{sync::mpsc, task::JoinSet, time::timeout};

use crate::connection::{Client, confirmed};

const REQUEST_BACKLOG: usize = 16;
/// Programs one connection runs at once; further requests wait their turn.
const CONCURRENT_PROGRAMS: usize = 8;
/// How long the host may take to start a program.
const START_TIMEOUT: Duration = Duration::from_secs(15);

struct Job {
    request: ExecRequest,
    answer: oneshot::Sender<Result<ExecOutput, ExecError>>,
}

/// Runs programs on the host of one SSH session.
#[derive(Clone)]
pub(crate) struct SshExec {
    requests: mpsc::Sender<Job>,
}

/// The connection's end of [`SshExec`].
pub(crate) struct ExecRequests {
    requests: mpsc::Receiver<Job>,
}

pub(crate) fn channel() -> (SshExec, ExecRequests) {
    let (requests, receiver) = mpsc::channel(REQUEST_BACKLOG);
    (SshExec { requests }, ExecRequests { requests: receiver })
}

impl HostExec for SshExec {
    fn exec(&self, request: ExecRequest) -> ExecFuture<ExecOutput> {
        let requests = self.requests.clone();
        Box::pin(async move {
            let (answer, answered) = oneshot::channel();
            requests
                .send(Job { request, answer })
                .await
                .map_err(|_| ExecError::Disconnected)?;
            answered.await.map_err(|_| ExecError::Disconnected)?
        })
    }
}

/// Starts requested programs until shutdown. Programs still running then are
/// dropped with their channels, which the host sees as a hang-up.
pub(crate) async fn serve(
    handle: Arc<Handle<Client>>,
    mut requests: ExecRequests,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let mut running = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => break,
            Some(_) = running.join_next(), if !running.is_empty() => {}
            job = requests.requests.recv(), if running.len() < CONCURRENT_PROGRAMS => {
                let Some(job) = job else { break };
                running.spawn(run(handle.clone(), job));
            }
        }
    }
    running.abort_all();
    while running.join_next().await.is_some() {}
}

async fn run(handle: Arc<Handle<Client>>, job: Job) {
    let Job { request, answer } = job;
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
    loop {
        tokio::select! {
            () = sink.closed() => break,
            message = channel.wait() => match message {
                Some(ChannelMsg::Data { data }) => {
                    if !sink.send(data.to_vec()).await {
                        break;
                    }
                }
                Some(ChannelMsg::Close | ChannelMsg::Eof) | None => return,
                Some(_) => {}
            },
        }
    }
    // Not every server honours signals; closing the channel also breaks the
    // program's output pipe.
    let _ = channel.signal(Sig::TERM).await;
    let _ = channel.close().await;
}
