//! Owns actual producer tasks and one shared shutdown deadline.
#[cfg(unix)]
mod credentials;
#[cfg(unix)]
mod health;
#[cfg(unix)]
mod worker;
use crate::{RunnerError, bootstrap::State, cli::Completion};
#[cfg(unix)]
use nocterm_core::persist::ShutdownDeadline;
#[cfg(unix)]
use nocterm_hub_io::cloud::RunnerHeadWork;
use std::path::Path;
#[cfg(unix)]
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

#[cfg(unix)]
pub(crate) async fn run(state: State) -> Completion {
    let budget = state.limits().shutdown();
    let operation = state.limits().operation();
    let restored = tokio::time::timeout(operation, state.connection().restore()).await;
    let peer = match restored {
        Ok(Ok(peer)) => peer,
        Ok(Err(error)) => return Completion::finished(Err(error.into()), budget),
        Err(_) => return Completion::finished(Err(RunnerError::Timeout), budget),
    };
    let work = match worker::work() {
        Ok(work) => work,
        Err(error) => return Completion::finished(Err(error), budget),
    };
    let ready = Arc::new(AtomicBool::new(false));
    let endpoint = match health::Endpoint::bind(state.directory()) {
        Ok(endpoint) => endpoint,
        Err(error) => return Completion::finished(Err(error), budget),
    };
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
    let mut observer = tokio::spawn(endpoint.serve(ready, work.clone()));
    let (result, producer_done, observer_done) = tokio::select! {
        signal = shutdown_signal() => (signal, false, false),
        result = &mut producer => (result.map_err(RunnerError::Task).and_then(|value| value), true, false),
        result = &mut observer => (result.map_err(RunnerError::Task).and_then(|value| value), false, true),
    };
    let deadline = ShutdownDeadline::new(budget);
    work.close();
    let _ = stop.send(());
    // Head waits may cancel. An already-started credential mutation gets the
    // same remaining shutdown deadline to finish instead of immediate abort.
    observer.abort();
    let producer_joined = producer_done
        || tokio::time::timeout(deadline.remaining(), &mut producer)
            .await
            .is_ok();
    if !producer_joined {
        producer.abort();
    }
    let retirement = tokio::task::spawn_blocking(move || connection.retire_local());
    let observer_joined = observer_done
        || tokio::time::timeout(deadline.remaining(), &mut observer)
            .await
            .is_ok();
    // Native retirement may wait for the private durable session writer. Keep
    // that real owner on the bounded blocking pool and account its actual join.
    let retired = matches!(
        tokio::time::timeout(deadline.remaining(), retirement).await,
        Ok(Ok(()))
    );
    let drained = producer_joined
        && observer_joined
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
pub(crate) async fn run(state: State) -> Completion {
    Completion::finished(Err(RunnerError::Platform), state.limits().shutdown())
}
#[cfg(not(unix))]
pub(crate) async fn health(_: &Path) -> Result<(), RunnerError> {
    Err(RunnerError::Platform)
}
