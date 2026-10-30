//! Native initial consent transitions to retained original-runtime authority.
use nocterm_agent_tools::headless::{
    InitialSessionCandidate, SessionLifecycleEvent, SessionObservation,
};
use nocterm_cloud_model::{
    authority::RunBinding,
    coordinator::{CoordinatorError, SessionFactSink},
    tool::ToolTimer,
};
use nocterm_core::SharedCause;
use nocterm_hub_io::cloud::RunnerHubConnection;
use nocterm_session::{ConnectionSource, ConnectionSourceError, ConnectionSourceGuard};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::watch;

enum Phase {
    Pending(Arc<dyn SessionFactSink>),
    Connected,
    Closed,
}
pub(crate) struct SessionConnectionScope {
    connection: Arc<RunnerHubConnection>,
    phase: Mutex<Phase>,
    changed: watch::Sender<()>,
    _residency: Arc<dyn Send + Sync>,
}
impl SessionConnectionScope {
    pub(crate) fn new(
        connection: Arc<RunnerHubConnection>,
        run: &RunBinding,
        facts: Arc<dyn SessionFactSink>,
        residency: Arc<dyn Send + Sync>,
    ) -> Result<Arc<Self>, CoordinatorError> {
        if !connection.binding().contains(&run.stream()) {
            return Err(CoordinatorError::StaleSource);
        }
        connection.check_runtime_source().map_err(backend)?;
        facts.check_current()?;
        let (changed, _) = watch::channel(());
        Ok(Arc::new(Self {
            connection,
            phase: Mutex::new(Phase::Pending(facts)),
            changed,
            _residency: residency,
        }))
    }
    pub(crate) fn source(self: &Arc<Self>) -> ConnectionSource {
        ConnectionSource::new(self.clone())
    }
    /// Only the sole actual reader's original Connected fact can release the
    /// initial run/window dependency. A durable receipt or restore cannot do so.
    pub(crate) fn connected(
        &self,
        candidate: &InitialSessionCandidate,
        observation: &SessionObservation,
    ) -> Result<(), CoordinatorError> {
        if observation.receipt() != candidate.receipt()
            || observation.generation() != candidate.generation()
            || !matches!(observation.event(), SessionLifecycleEvent::Connected)
        {
            return Err(CoordinatorError::Conflict);
        }
        self.connection.check_runtime_source().map_err(backend)?;
        let mut phase = self
            .phase
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?;
        let Phase::Pending(facts) = &*phase else {
            return Err(CoordinatorError::Conflict);
        };
        facts.check_current()?;
        *phase = Phase::Connected;
        self.changed.send_replace(());
        Ok(())
    }
    pub(crate) fn close(&self) {
        if let Ok(mut phase) = self.phase.lock() {
            *phase = Phase::Closed;
            self.changed.send_replace(());
        }
    }
    pub(crate) fn ended(
        self: &Arc<Self>,
        timer: Arc<dyn ToolTimer>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let scope = self.clone();
        Box::pin(async move {
            let mut changed = scope.changed.subscribe();
            loop {
                if scope.check().is_err() {
                    return;
                }
                let pending = match scope.phase.lock() {
                    Ok(phase) => match &*phase {
                        Phase::Pending(facts) => Some(facts.clone()),
                        _ => None,
                    },
                    Err(_) => return,
                };
                let initial_ended = async move {
                    match pending {
                        Some(facts) => facts.ended().await,
                        None => std::future::pending::<()>().await,
                    }
                };
                tokio::select! {
                    _ = initial_ended => return,
                    result = changed.changed() => if result.is_err() { return; },
                    _ = timer.delay(Duration::from_millis(100)) => {}
                }
            }
        })
    }
}
impl ConnectionSourceGuard for SessionConnectionScope {
    fn check(&self) -> Result<(), ConnectionSourceError> {
        self.connection
            .check_runtime_source()
            .map_err(|_| ConnectionSourceError::Revoked)?;
        let phase = self
            .phase
            .lock()
            .map_err(|_| ConnectionSourceError::Unavailable)?;
        match &*phase {
            Phase::Pending(facts) => facts
                .check_current()
                .map_err(|_| ConnectionSourceError::Revoked),
            Phase::Connected => Ok(()),
            Phase::Closed => Err(ConnectionSourceError::Revoked),
        }
    }
}
fn backend(error: impl std::error::Error + Send + Sync + 'static) -> CoordinatorError {
    CoordinatorError::Backend(SharedCause::new(error))
}
