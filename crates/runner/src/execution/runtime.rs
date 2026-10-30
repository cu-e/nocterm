//! The running execution owner: recovery, boot announcement, command service,
//! outbox delivery and bounded drain. It is created only from complete
//! explicit configuration and never outlives the daemon's native control plane.
use super::{compose::compose, publisher::OutboxPublisher};
use crate::{RunnerError, bootstrap::State, sessions::NativeSessionBroker};
use futures::future::{AbortHandle, AbortRegistration};
use nocterm_cloud_model::{
    coordinator::{CoordinatorError, DrainReport},
    journal::{PublicationAllocation, PublicationWorkBudget},
    tool::ToolTimer,
};
use nocterm_cloud_runner::{DurableRunJournal, coordinator::RunCoordinator};
use nocterm_core::persist::ShutdownDeadline;
use nocterm_hub_io::cloud::{RunnerCommandAttachments, RunnerHubPeer, provider_boot_work_bytes};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::task::JoinHandle;

type Task = JoinHandle<Result<(), CoordinatorError>>;

struct Running {
    abort: AbortHandle,
    serving: Task,
    publishing: Task,
}
pub(crate) struct ExecutionRuntime {
    coordinator: RunCoordinator,
    broker: Arc<NativeSessionBroker>,
    attachments: Arc<RunnerCommandAttachments>,
    journal: DurableRunJournal,
    timer: Arc<dyn ToolTimer>,
    transport: Arc<dyn PublicationWorkBudget>,
    operation: Duration,
    ready: Arc<AtomicBool>,
    running: Option<Running>,
}
impl ExecutionRuntime {
    pub(super) fn new(
        coordinator: RunCoordinator,
        broker: Arc<NativeSessionBroker>,
        attachments: Arc<RunnerCommandAttachments>,
        journal: DurableRunJournal,
        timer: Arc<dyn ToolTimer>,
        transport: Arc<dyn PublicationWorkBudget>,
        operation: Duration,
    ) -> Self {
        Self {
            coordinator,
            broker,
            attachments,
            journal,
            timer,
            transport,
            operation,
            ready: Arc::new(AtomicBool::new(false)),
            running: None,
        }
    }
    /// Composes, recovers the durable catalog, announces this boot and starts
    /// serving. Any failure leaves nothing running and refuses the daemon.
    pub(crate) async fn start(
        setup: super::ExecutionSetup,
        state: &State,
        peer: RunnerHubPeer,
    ) -> Result<Self, RunnerError> {
        let mut runtime = compose(setup, state, peer).await?;
        let begun = runtime.begin().await;
        match begun {
            Ok(()) => Ok(runtime),
            Err(error) => {
                runtime.fail_closed();
                Err(error)
            }
        }
    }
    async fn begin(&mut self) -> Result<(), RunnerError> {
        self.bounded(self.coordinator.recover()).await?;
        let lease = self
            .transport
            .try_reserve(PublicationAllocation::new(provider_boot_work_bytes(), 1)?)?;
        self.bounded(self.attachments.announce_boot(lease)).await?;
        let (abort, registration) = AbortHandle::new_pair();
        let serving = tokio::spawn(serve(self.coordinator.clone(), registration));
        let publisher = OutboxPublisher::new(
            self.coordinator.clone(),
            self.journal.clone(),
            self.timer.clone(),
        );
        let publishing = tokio::spawn(publisher.run());
        self.running = Some(Running {
            abort,
            serving,
            publishing,
        });
        self.ready.store(true, Ordering::Release);
        Ok(())
    }
    async fn bounded<T>(
        &self,
        work: impl std::future::Future<Output = Result<T, CoordinatorError>>,
    ) -> Result<T, RunnerError> {
        Ok(tokio::time::timeout(self.operation, work)
            .await
            .map_err(|_| RunnerError::Timeout)??)
    }
    /// True only while the coordinator is serving and its boot is still valid.
    pub(crate) fn readiness(&self) -> Arc<AtomicBool> {
        self.ready.clone()
    }
    /// Completes when execution can no longer serve. A boot window that has
    /// expired is fatal: devices can no longer deliver provider credentials to
    /// it, so the supervisor must start a fresh boot instead of this one.
    pub(crate) async fn stopped(&mut self) -> Result<(), RunnerError> {
        let remaining = self.boot_remaining();
        let Some(running) = self.running.as_mut() else {
            return Err(RunnerError::Execution(CoordinatorError::Stopped));
        };
        let outcome = tokio::select! {
            result = &mut running.serving => joined(result),
            result = &mut running.publishing => joined(result),
            _ = self.timer.delay(remaining) => Err(RunnerError::Expired),
        };
        self.ready.store(false, Ordering::Release);
        outcome
    }
    fn boot_remaining(&self) -> Duration {
        let expires = self
            .attachments
            .boot_offer()
            .expires()
            .map_or(0, |value| value.as_millis());
        Duration::from_millis(expires.saturating_sub(self.timer.now().as_millis()))
    }
    fn fail_closed(&mut self) {
        self.ready.store(false, Ordering::Release);
        self.coordinator.fence();
        if let Some(running) = self.running.take() {
            running.abort.abort();
            running.publishing.abort();
            running.serving.abort();
        }
        self.broker.close();
    }
    /// Stops fresh effects, then waits for actual admitted work and native
    /// session owners within the single shared deadline.
    pub(crate) async fn drain(mut self, deadline: ShutdownDeadline) -> bool {
        self.ready.store(false, Ordering::Release);
        self.coordinator.fence();
        if let Some(running) = self.running.take() {
            running.abort.abort();
            running.publishing.abort();
            let mut serving = running.serving;
            if tokio::time::timeout(deadline.remaining(), &mut serving)
                .await
                .is_err()
            {
                serving.abort();
            }
        }
        let coordinator = matches!(
            self.coordinator.shutdown(deadline).await,
            Ok(DrainReport::Drained)
        );
        let broker = self.broker.drain(deadline).await;
        coordinator && broker
    }
}
async fn serve(
    coordinator: RunCoordinator,
    registration: AbortRegistration,
) -> Result<(), CoordinatorError> {
    coordinator.serve(registration).await
}
fn joined(
    result: Result<Result<(), CoordinatorError>, tokio::task::JoinError>,
) -> Result<(), RunnerError> {
    result.map_err(RunnerError::Task)?.map_err(Into::into)
}
