//! Agent startup as consecutive typed stages; each consumes the previous stage's output.
use std::{path::PathBuf, sync::Arc};

use futures::{
    channel::oneshot,
    future::{Either, select},
};
use nocterm_ai::{AgentConnection, AgentError, ConnectRequest, ConnectionCancellation};

use super::{
    driver::{Driver, Ready},
    error::StartupError,
    exit_report::EventQueue,
    handshake::Handshake,
    pipes::Pipes,
    stderr_tail::StderrTail,
};
use crate::process::{self, ProcessGroup, Spawned};

/// Bounded protocol events before a slow consumer is reported as an overflow.
const EVENT_CAPACITY: usize = 256;

/// Starts the agent described by `request` and returns once it has initialized.
pub(super) async fn connect(
    request: ConnectRequest,
    helper: Option<PathBuf>,
) -> Result<AgentConnection, AgentError> {
    let launched = Launched::spawn(request, helper.as_deref())?;
    let piped = launched.attach_pipes()?;
    let driven = piped.start_driver()?;
    Ok(driven.initialized().await?)
}

/// Kills a half-started process tree unless startup completes.
struct ConnectingGuard(Option<Arc<ProcessGroup>>);
impl ConnectingGuard {
    fn defuse(&mut self) {
        self.0.take();
    }
}
impl Drop for ConnectingGuard {
    fn drop(&mut self) {
        if let Some(group) = &self.0 {
            group.kill();
        }
    }
}

/// What startup needs from the request after the process exists.
struct Identity {
    name: String,
    terminal_auth: bool,
    cancellation: ConnectionCancellation,
}

/// Stage one: the subprocess is running and guarded.
struct Launched {
    process: Spawned,
    guard: ConnectingGuard,
    identity: Identity,
}
impl Launched {
    fn spawn(
        request: ConnectRequest,
        helper: Option<&std::path::Path>,
    ) -> Result<Self, StartupError> {
        if request.cancellation.is_cancelled() {
            return Err(StartupError::Cancelled);
        }
        let process = process::spawn(&request, helper)?;
        Ok(Self {
            guard: ConnectingGuard(Some(process.group.clone())),
            process,
            identity: Identity {
                name: request.launch.name,
                terminal_auth: request.terminal_auth,
                cancellation: request.cancellation,
            },
        })
    }
    fn attach_pipes(mut self) -> Result<Piped, StartupError> {
        let pipes = Pipes::take(&mut self.process.child)?;
        Ok(Piped {
            launched: self,
            pipes,
        })
    }
}

/// Stage two: the standard streams are in hand.
struct Piped {
    launched: Launched,
    pipes: Pipes,
}
impl Piped {
    fn start_driver(self) -> Result<Driven, StartupError> {
        let Launched {
            process,
            guard,
            identity,
        } = self.launched;
        let (stdin, stdout, stderr) = self.pipes.split();
        let tail = StderrTail::capture(stderr)?;
        let group = process.group.clone();
        let handshake = Handshake::new(
            identity.name,
            identity.terminal_auth,
            process.secrets.clone(),
        );
        let (ready, answer): (Ready, _) = oneshot::channel();
        Driver::new(
            process,
            (stdin, stdout),
            tail,
            EventQueue::new(EVENT_CAPACITY),
            handshake,
            ready,
        )
        .spawn()?;
        Ok(Driven {
            answer,
            group,
            guard,
            cancellation: identity.cancellation,
        })
    }
}

/// Stage three: the driver thread is handshaking.
struct Driven {
    answer: oneshot::Receiver<Result<AgentConnection, StartupError>>,
    group: Arc<ProcessGroup>,
    guard: ConnectingGuard,
    cancellation: ConnectionCancellation,
}
impl Driven {
    async fn initialized(mut self) -> Result<AgentConnection, StartupError> {
        let outcome = match select(Box::pin(self.answer), self.cancellation.cancelled()).await {
            Either::Left((answer, _)) => {
                answer.unwrap_or(Err(StartupError::ExitedDuringInitialization))
            }
            Either::Right(_) => Err(StartupError::Cancelled),
        };
        if outcome.is_ok() {
            self.guard.defuse();
        } else {
            self.group
                .shutdown_confirmed()
                .await
                .map_err(StartupError::Cleanup)?;
        }
        outcome
    }
}
