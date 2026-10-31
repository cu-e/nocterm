//! Owns actual producer tasks and one shared shutdown deadline.
#[cfg(unix)]
mod credentials;
#[cfg(unix)]
mod health;
#[cfg(unix)]
mod worker;
#[cfg(unix)]
use crate::execution::ExecutionRuntime;
use crate::{RunnerError, bootstrap::State, cli::Completion, managed::Session};
#[cfg(unix)]
use nocterm_core::persist::ShutdownDeadline;
#[cfg(unix)]
use nocterm_hub_io::cloud::{RunnerHeadWork, RunnerHubConnection};
use std::path::Path;
#[cfg(unix)]
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
#[cfg(unix)]
use tokio::task::JoinHandle;

/// Everything the daemon needs before it may report itself running. Execution
/// is started last, so a refused containment or boot leaves no producer behind.
#[cfg(unix)]
struct Startup {
    peer: nocterm_hub_io::cloud::RunnerHubPeer,
    work: Arc<RunnerHeadWork>,
    endpoint: health::Endpoint,
    execution: Option<ExecutionRuntime>,
}
#[cfg(unix)]
async fn startup(state: &mut State) -> Result<Startup, RunnerError> {
    let operation = state.limits().operation();
    let peer = tokio::time::timeout(operation, state.connection().restore())
        .await
        .map_err(|_| RunnerError::Timeout)??;
    let work = worker::work()?;
    let endpoint = health::Endpoint::bind(state.directory())?;
    let execution = match state.take_execution() {
        Some(setup) => Some(
            tokio::time::timeout(
                operation,
                ExecutionRuntime::start(setup, state, peer.clone()),
            )
            .await
            .map_err(|_| RunnerError::Timeout)??,
        ),
        None => None,
    };
    Ok(Startup {
        peer,
        work,
        endpoint,
        execution,
    })
}
#[cfg(unix)]
pub(crate) async fn run(mut state: State, session: Option<Session>) -> Completion {
    let budget = state.limits().shutdown();
    match startup(&mut state).await {
        Ok(started) => {
            if let Some(session) = &session
                && let Err(error) = session.announce(started.execution.is_some())
            {
                return Completion::finished(Err(error), budget);
            }
            supervise(state, started, session).await
        }
        Err(error) => Completion::finished(Err(error), budget),
    }
}
/// The host closing the control channel ends the lease, so the runner stops
/// like it does for a signal. A daemon without a channel never stops for it.
#[cfg(unix)]
async fn control_ended(session: &mut Option<Session>) -> Result<(), RunnerError> {
    match session {
        Some(session) => session.ended().await,
        None => std::future::pending().await,
    }
}
#[cfg(unix)]
async fn supervise(state: State, started: Startup, mut session: Option<Session>) -> Completion {
    let Startup {
        peer,
        work,
        endpoint,
        mut execution,
    } = started;
    let budget = state.limits().shutdown();
    let ready = Arc::new(AtomicBool::new(false));
    let executing = execution.as_ref().map_or_else(
        || Arc::new(AtomicBool::new(false)),
        ExecutionRuntime::readiness,
    );
    let connection = state.connection().clone();
    let credentials = Arc::new(credentials::CredentialOperations::default());
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let mut producer = tokio::spawn(worker::serve(
        state,
        peer,
        work.clone(),
        ready.clone(),
        credentials.clone(),
        stopped,
    ));
    let mut observer = tokio::spawn(endpoint.serve(ready, executing, work.clone()));
    let (result, producer_done, observer_done) = tokio::select! {
        signal = shutdown_signal() => (signal, false, false),
        result = &mut producer => (flatten(result), true, false),
        result = &mut observer => (flatten(result), false, true),
        result = execution_stopped(&mut execution) => (result, false, false),
        result = control_ended(&mut session) => (result, false, false),
    };
    let deadline = ShutdownDeadline::new(budget);
    work.close();
    let _ = stop.send(());
    // Head waits may cancel. An already-started credential mutation gets the
    // same remaining shutdown deadline to finish instead of immediate abort.
    observer.abort();
    let executed = match execution {
        Some(execution) => execution.drain(deadline).await,
        None => true,
    };
    let joined = join(&mut producer, producer_done, &deadline).await;
    let retired = retire(&connection, &deadline).await;
    let observed = join(&mut observer, observer_done, &deadline).await;
    let drained = executed
        && joined
        && observed
        && retired
        && credentials.completed()
        && wait_for_work(&work, &deadline).await;
    Completion::drained(
        if drained {
            result
        } else {
            Err(RunnerError::DrainTimeout)
        },
        deadline,
    )
}
#[cfg(unix)]
fn flatten(
    result: Result<Result<(), RunnerError>, tokio::task::JoinError>,
) -> Result<(), RunnerError> {
    result.map_err(RunnerError::Task).and_then(|value| value)
}
/// A daemon without execution never stops because of it.
#[cfg(unix)]
async fn execution_stopped(execution: &mut Option<ExecutionRuntime>) -> Result<(), RunnerError> {
    match execution {
        Some(execution) => execution.stopped().await,
        None => std::future::pending().await,
    }
}
#[cfg(unix)]
async fn join(
    task: &mut JoinHandle<Result<(), RunnerError>>,
    done: bool,
    deadline: &ShutdownDeadline,
) -> bool {
    if done
        || tokio::time::timeout(deadline.remaining(), &mut *task)
            .await
            .is_ok()
    {
        return true;
    }
    task.abort();
    false
}
// Native retirement may wait for the private durable session writer. Keep that
// real owner on the bounded blocking pool and account its actual join.
#[cfg(unix)]
async fn retire(connection: &Arc<RunnerHubConnection>, deadline: &ShutdownDeadline) -> bool {
    let connection = connection.clone();
    let retirement = tokio::task::spawn_blocking(move || connection.retire_local());
    matches!(
        tokio::time::timeout(deadline.remaining(), retirement).await,
        Ok(Ok(()))
    )
}
#[cfg(unix)]
async fn wait_for_work(work: &RunnerHeadWork, deadline: &ShutdownDeadline) -> bool {
    loop {
        if work.in_flight_reads() == 0 && work.in_flight_operations() == 0 {
            return true;
        }
        if deadline.remaining().is_zero() {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10).min(deadline.remaining())).await;
    }
}
#[cfg(unix)]
async fn shutdown_signal() -> Result<(), RunnerError> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut interrupt = signal(SignalKind::interrupt()).map_err(RunnerError::Control)?;
    let mut terminate = signal(SignalKind::terminate()).map_err(RunnerError::Control)?;
    tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
    Ok(())
}
#[cfg(unix)]
pub(crate) async fn health(directory: &Path) -> Result<(), RunnerError> {
    health::query(directory).await
}
#[cfg(not(unix))]
pub(crate) async fn run(state: State, _session: Option<Session>) -> Completion {
    Completion::finished(Err(RunnerError::Platform), state.limits().shutdown())
}
#[cfg(not(unix))]
pub(crate) async fn health(_: &Path) -> Result<(), RunnerError> {
    Err(RunnerError::Platform)
}
