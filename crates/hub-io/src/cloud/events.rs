//! Purpose-bound public observations from original native coordinator facts.
mod arguments;
pub use arguments::RunnerArgumentFingerprint;
mod mapping;
mod prompt;
mod sealing;
mod snapshot;
pub(super) mod ssh;
mod tools;
pub(super) mod values;
use super::RunnerJournalCrypto;
use nocterm_cloud_model::{
    coordinator::{CoordinatorEvent, CoordinatorEventEncoder},
    journal::{
        BackgroundRecordOutcome, JournalError, JournalEventEncoder, JournalPosition,
        PublicationAllocation, PublicationWorkBudget, ResidentPublication, WriterLease,
    },
    tool::BackgroundCommand,
};
use nocterm_core::{SharedCause, clock::WallClock};
use nocterm_hub_model::cloud::events::{
    BackgroundLifecyclePayload, BackgroundOutcome, CloudEvent, CloudEventPayload,
};
use std::sync::Arc;

/// Owns configured material and clock; network delivery consumes the persisted
/// result separately. No provider, filesystem or target operation runs here.
pub struct RunnerEventEncoder {
    material: Arc<RunnerJournalCrypto>,
    clock: Arc<dyn WallClock>,
}
impl RunnerEventEncoder {
    pub fn new(material: Arc<RunnerJournalCrypto>, clock: Arc<dyn WallClock>) -> Self {
        Self { material, clock }
    }
    fn timestamp(&self) -> Result<u64, JournalError> {
        u64::try_from(self.clock.now().unix_seconds)
            .ok()
            .and_then(|seconds| seconds.checked_mul(1000))
            .filter(|value| *value > 0)
            .ok_or(JournalError::Corrupt)
    }
}
impl CoordinatorEventEncoder for RunnerEventEncoder {
    fn encode(
        &self,
        lease: &WriterLease,
        next: JournalPosition,
        event: &CoordinatorEvent,
        work: &dyn PublicationWorkBudget,
    ) -> Result<ResidentPublication, JournalError> {
        let _mapping = work.try_reserve(PublicationAllocation::new(
            mapping::mapping_bytes(event)?,
            1,
        )?)?;
        let timestamp_ms = self.timestamp()?;
        mapping::with_payload(event, work, |payload| {
            sealing::RecordSealer::new(&self.material, work).seal(
                lease,
                next,
                mapping::original_run(event),
                sealing::RecordPayload::Event(CloudEvent {
                    control_revision: next.control().get(),
                    timestamp_ms,
                    payload,
                }),
            )
        })
    }
}
impl JournalEventEncoder for RunnerEventEncoder {
    fn encode(
        &self,
        lease: &WriterLease,
        next: JournalPosition,
        original: &BackgroundCommand,
        outcome: BackgroundRecordOutcome,
        work: &dyn PublicationWorkBudget,
    ) -> Result<ResidentPublication, JournalError> {
        let intent = original.dispatch().intent();
        let bytes = std::mem::size_of::<BackgroundLifecyclePayload<'static>>()
            .checked_add(
                intent
                    .authority()
                    .session()
                    .target()
                    .principal()
                    .as_str()
                    .len(),
            )
            .ok_or(JournalError::Capacity)?;
        let _mapping = work.try_reserve(PublicationAllocation::new(bytes, 1)?)?;
        let observation = BackgroundLifecyclePayload {
            job: mapping::background(original)?,
            step: *intent.step().as_bytes(),
            arguments_digest: *intent.arguments().as_bytes(),
            mutates: intent.mutates(),
            dispatch_writer_epoch: original.dispatch().commit().writer().get(),
            dispatch_revision: original.dispatch().commit().revision().get(),
            outcome: match outcome {
                BackgroundRecordOutcome::Retained => BackgroundOutcome::Retained,
                BackgroundRecordOutcome::ObservedExit(exit) => {
                    BackgroundOutcome::ObservedExit(exit)
                }
                BackgroundRecordOutcome::Unknown => BackgroundOutcome::Unknown,
            },
        };
        sealing::RecordSealer::new(&self.material, work).seal(
            lease,
            next,
            Some(intent.authority().run()),
            sealing::RecordPayload::Event(CloudEvent {
                control_revision: next.control().get(),
                timestamp_ms: self.timestamp()?,
                payload: CloudEventPayload::BackgroundLifecycle(Box::new(observation)),
            }),
        )
    }
}
impl nocterm_cloud_model::journal::CoordinatorSnapshotEncoder for RunnerEventEncoder {
    fn encode(
        &self,
        lease: &WriterLease,
        next: JournalPosition,
        source: &nocterm_cloud_model::journal::SnapshotProjectionSource,
        limits: nocterm_cloud_model::journal::SnapshotLimits,
        work: &dyn PublicationWorkBudget,
    ) -> Result<ResidentPublication, JournalError> {
        snapshot::SnapshotProjection::new(&self.material, work).encode(
            lease,
            next,
            source,
            limits,
            self.timestamp()?,
        )
    }
}

fn encoding_error(error: impl std::error::Error + Send + Sync + 'static) -> JournalError {
    JournalError::Storage(SharedCause::new(error))
}

#[cfg(test)]
mod tests;
