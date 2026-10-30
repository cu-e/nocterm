//! Finite process/account pools shared by their actual foreground and native owners.
use crate::RunnerError;
use nocterm_cloud_model::{
    coordinator::{CommandReceiveBudget, IntakeLimits, IntakePoolLimits},
    journal::{PublicationBudget, PublicationWorkBudget},
};
use nocterm_cloud_runner::CoordinatorPublicationBudgets;
use std::sync::Arc;

pub(super) struct ExecutionBudgets {
    receive: CommandReceiveBudget,
    publication: CoordinatorPublicationBudgets,
    transport: Arc<dyn PublicationWorkBudget>,
    snapshots: Arc<dyn PublicationWorkBudget>,
    attachments: Arc<dyn PublicationWorkBudget>,
    sessions: Arc<dyn PublicationWorkBudget>,
    capture: Arc<dyn PublicationWorkBudget>,
    arguments: Arc<dyn PublicationWorkBudget>,
}
impl ExecutionBudgets {
    pub(super) fn new(cleanup_bytes: usize) -> Result<Self, RunnerError> {
        let receive = CommandReceiveBudget::new(
            IntakeLimits::new(
                limits(64 * 1024 * 1024, 4)?,
                limits(64 * 1024 * 1024, 32)?,
                limits(2 * 1024 * 1024, 16)?,
                limits(1024 * 1024, 64)?,
                limits(256 * 1024, 16)?,
            )
            .map_err(|_| RunnerError::Value("execution.intake"))?,
        );
        Ok(Self {
            publication: CoordinatorPublicationBudgets::new(
                receive.clone(),
                limits(
                    cleanup_bytes
                        .checked_add(32 * 1024 * 1024)
                        .ok_or(RunnerError::Value("execution.cleanup work"))?,
                    32,
                )?,
            ),
            receive,
            transport: pool(16 * 1024 * 1024, 16)?,
            snapshots: pool(256 * 1024 * 1024, 4)?,
            attachments: pool(8 * 1024 * 1024, 64)?,
            sessions: pool(8 * 1024 * 1024, 128)?,
            capture: pool(32 * 1024 * 1024, 512)?,
            arguments: pool(2 * 1024 * 1024, 32)?,
        })
    }
    pub(super) fn receive(&self) -> CommandReceiveBudget {
        self.receive.clone()
    }
    pub(super) fn publication(&self) -> CoordinatorPublicationBudgets {
        self.publication.clone()
    }
    pub(super) fn transport(&self) -> Arc<dyn PublicationWorkBudget> {
        self.transport.clone()
    }
    pub(super) fn snapshots(&self) -> Arc<dyn PublicationWorkBudget> {
        self.snapshots.clone()
    }
    pub(super) fn attachments(&self) -> Arc<dyn PublicationWorkBudget> {
        self.attachments.clone()
    }
    pub(super) fn sessions(&self) -> Arc<dyn PublicationWorkBudget> {
        self.sessions.clone()
    }
    pub(super) fn capture(&self) -> Arc<dyn PublicationWorkBudget> {
        self.capture.clone()
    }
    pub(super) fn arguments(&self) -> Arc<dyn PublicationWorkBudget> {
        self.arguments.clone()
    }
}
fn limits(bytes: usize, payloads: usize) -> Result<IntakePoolLimits, RunnerError> {
    IntakePoolLimits::new(bytes, payloads, payloads)
        .map_err(|_| RunnerError::Value("execution.work limits"))
}
fn pool(bytes: usize, payloads: usize) -> Result<Arc<dyn PublicationWorkBudget>, RunnerError> {
    Ok(Arc::new(PublicationBudget::maintenance(limits(
        bytes, payloads,
    )?)))
}
