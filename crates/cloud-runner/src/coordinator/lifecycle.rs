//! An admitted foreground reservation survives cancellation until retirement.
use super::{chat::Chat, queue::RunQueue, run::RunRecord, services::CoordinatorServices};
use futures::FutureExt;
use nocterm_cloud_model::{
    admission::AdmissionLease,
    coordinator::{CoordinatorError, RunStatus},
    tool::RunRetirement,
};
use std::sync::Arc;

pub(super) struct WorkerLease {
    services: Arc<CoordinatorServices>,
    queue: Arc<RunQueue>,
    lease: Option<AdmissionLease>,
    work: Option<(Arc<Chat>, Arc<RunRecord>)>,
}
impl WorkerLease {
    pub(super) fn new(
        services: Arc<CoordinatorServices>,
        queue: Arc<RunQueue>,
        lease: AdmissionLease,
    ) -> Self {
        Self {
            services,
            queue,
            lease: Some(lease),
            work: None,
        }
    }
    pub(super) fn attach(&mut self, chat: Arc<Chat>, run: Arc<RunRecord>) {
        self.work = Some((chat, run));
    }
    pub(super) fn finish(mut self) -> Result<(), CoordinatorError> {
        let Some(lease) = self.lease.take() else {
            return Ok(());
        };
        self.queue.release(&lease)
    }
}
impl Drop for WorkerLease {
    fn drop(&mut self) {
        let Some(lease) = self.lease.take() else {
            return;
        };
        let reservation = Reservation {
            queue: self.queue.clone(),
            lease,
        };
        let Some((chat, run)) = self.work.take() else {
            return;
        };
        // This synchronous barrier runs even if the spawned cleanup cannot be
        // polled during executor shutdown. It never grants another effect.
        run.control().fence();
        let failed = run.clone();
        let work = async move {
            let _reservation = reservation;
            let status = run.state().lock().await.status();
            if chat.check().is_ok()
                && !matches!(
                    status,
                    RunStatus::Completed | RunStatus::Stopped | RunStatus::OutcomeUnknown
                )
            {
                let _ = run.status(chat.log(), RunStatus::Interrupted).await;
            }
            // The reservation remains charged while the original runtime is
            // retiring. Dropping the outer drive/serve future cannot skip it.
            run.control().retire(RunRetirement::Interrupted).await
        }
        .boxed();
        if self.services.tasks().retire(work).is_err() {
            failed.control().stop();
            self.services.tasks().note_failure();
        }
    }
}
struct Reservation {
    queue: Arc<RunQueue>,
    lease: AdmissionLease,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let _ = self.queue.release(&self.lease);
    }
}

/// The receiver's outer lifetime also fences completed foreground/background runs.
/// It retains narrow collaborators rather than a reference back to the actor.
pub(super) struct ServingLease {
    services: Arc<CoordinatorServices>,
    queue: Arc<RunQueue>,
}
impl ServingLease {
    pub(super) fn new(services: Arc<CoordinatorServices>, queue: Arc<RunQueue>) -> Self {
        Self { services, queue }
    }
}
impl Drop for ServingLease {
    fn drop(&mut self) {
        self.services.tasks().begin_drain();
        for binding in self.services.stop_runs() {
            self.queue.cancel(&binding);
        }
    }
}
