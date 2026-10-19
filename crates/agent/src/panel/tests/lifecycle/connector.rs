//! Pending startup uses the original cancellation and observation counters.
use super::*;
struct CancelGuard(Arc<AtomicUsize>);
impl Drop for CancelGuard {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
pub(super) struct DelayedConnector {
    started: Arc<AtomicUsize>,
    cancelled: Arc<AtomicUsize>,
}
impl DelayedConnector {
    pub(super) fn new(started: Arc<AtomicUsize>, cancelled: Arc<AtomicUsize>) -> Self {
        Self { started, cancelled }
    }
}
#[derive(Debug, thiserror::Error)]
#[error("I/O error: Startup cancelled")]
struct StartupCancelled;
impl AgentConnector for DelayedConnector {
    fn connect_managed(
        &self,
        request: nocterm_ai::connection::ManagedConnectRequest,
    ) -> Result<
        nocterm_ai::history::source::ManagedSourceFuture<
            Result<
                nocterm_ai::connection::ManagedAgentConnection,
                nocterm_ai::history::source::ManagedCommandError,
            >,
        >,
        nocterm_ai::model::json::deferred::NativeAllocationError,
    > {
        let ticket = request.context().prepare_error::<StartupCancelled>(0)?;
        let lifetime = request.lifetime();
        let cancellation = request
            .cancellation()
            .cancelled_managed()
            .expect("cancellation wait admitted");
        let started = self.started.clone();
        let cancelled = self.cancelled.clone();
        nocterm_ai::history::source::ManagedSourceFuture::new(lifetime, async move {
            started.fetch_add(1, Ordering::SeqCst);
            let _guard = CancelGuard(cancelled);
            cancellation.await;
            Err(nocterm_ai::history::source::ManagedCommandError::Native(
                ticket.capture(StartupCancelled),
            ))
        })
    }
}
