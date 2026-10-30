//! Physical capabilities wrap actual candidates, never descriptive receipts.
use nocterm_agent_tools::headless::{InitialSessionCandidate, PublishedInitialSession};
use nocterm_cloud_model::{
    authority::SessionReceipt,
    coordinator::{
        CoordinatorError, PreparedRunSession, PublishedRunSession, SessionAttemptCancellation,
    },
};
use nocterm_core::SharedCause;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

pub(super) struct PhysicalCandidate {
    receipt: SessionReceipt,
    candidate: Arc<Mutex<Option<InitialSessionCandidate>>>,
}
pub(super) struct CandidateCancellation(Arc<Mutex<Option<InitialSessionCandidate>>>);
impl PhysicalCandidate {
    pub(super) fn new(candidate: InitialSessionCandidate) -> (Self, CandidateCancellation) {
        let receipt = candidate.receipt().clone();
        let candidate = Arc::new(Mutex::new(Some(candidate)));
        (
            Self {
                receipt,
                candidate: candidate.clone(),
            },
            CandidateCancellation(candidate),
        )
    }
}
impl CandidateCancellation {
    pub(super) fn cancel(&self) {
        retire_pending(&self.0);
    }
}
impl Drop for PhysicalCandidate {
    fn drop(&mut self) {
        retire_pending(&self.candidate);
    }
}
fn retire_pending(owner: &Mutex<Option<InitialSessionCandidate>>) {
    let candidate = owner
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    // Release the lock before native retirement. A publication already taken by
    // the sink owns its separate conditional guard; this becomes a no-op.
    drop(candidate);
}
impl PreparedRunSession for PhysicalCandidate {
    fn receipt(&self) -> &SessionReceipt {
        &self.receipt
    }
    fn publish(self: Box<Self>) -> Result<Box<dyn PublishedRunSession>, CoordinatorError> {
        let candidate = self
            .candidate
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?
            .take()
            .ok_or(CoordinatorError::Stopped)?;
        Ok(Box::new(PhysicalPublication(
            candidate.publish_owned().map_err(backend)?,
        )))
    }
}
struct PhysicalPublication(PublishedInitialSession);
impl PublishedRunSession for PhysicalPublication {
    fn receipt(&self) -> &SessionReceipt {
        self.0.receipt()
    }
    fn accept(self: Box<Self>) {
        let Self(publication) = *self;
        publication.accept();
    }
}
enum PendingPhase {
    Pending,
    Connected,
    Cancelled,
}
pub(super) struct PendingConnection {
    phase: Mutex<PendingPhase>,
    changed: watch::Sender<()>,
}
impl PendingConnection {
    pub(super) fn new() -> Arc<Self> {
        let (changed, _) = watch::channel(());
        Arc::new(Self {
            phase: Mutex::new(PendingPhase::Pending),
            changed,
        })
    }
    pub(super) fn check(&self) -> Result<(), CoordinatorError> {
        let phase = self
            .phase
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?;
        if matches!(*phase, PendingPhase::Cancelled) {
            Err(CoordinatorError::Stopped)
        } else {
            Ok(())
        }
    }
    pub(super) fn connected(&self) -> Result<(), CoordinatorError> {
        let mut phase = self
            .phase
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?;
        if !matches!(*phase, PendingPhase::Pending) {
            return Err(CoordinatorError::Stopped);
        }
        *phase = PendingPhase::Connected;
        self.changed.send_replace(());
        Ok(())
    }
    pub(super) async fn cancelled(&self) {
        let mut changed = self.changed.subscribe();
        loop {
            if self.check().is_err() {
                return;
            }
            if changed.changed().await.is_err() {
                return;
            }
        }
    }
}
pub(super) struct CancelPending(Arc<PendingConnection>);
impl CancelPending {
    pub(super) fn new(pending: Arc<PendingConnection>) -> Self {
        Self(pending)
    }
}
impl SessionAttemptCancellation for CancelPending {
    fn cancel(&self) {
        if let Ok(mut phase) = self.0.phase.lock()
            && matches!(*phase, PendingPhase::Pending)
        {
            *phase = PendingPhase::Cancelled;
            self.0.changed.send_replace(());
        }
    }
}
fn backend(error: impl std::error::Error + Send + Sync + 'static) -> CoordinatorError {
    CoordinatorError::Backend(SharedCause::new(error))
}
