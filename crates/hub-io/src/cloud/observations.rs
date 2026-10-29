//! Pinned account material opens canonical signed observations before mapping.
//! Owns: bounded native decode and source fencing. Does not own: replay freshness,
//! chain discovery, network I/O, execution authority or snapshot activation.
mod budget;
mod mapping;
mod prompt;
mod tools;
mod values;
mod work;

use super::{AccountChatMaterial, AccountMaterialError, runner_transport::record_context};
use nocterm_cloud_model::{
    identity::{ChatControlRevision, Timestamp},
    journal::{HeadAnchor, HeadAnchorKind},
    projection::{
        ProjectionBinding, ProjectionCursor, ProjectionError, ProjectionEvent, SnapshotBaseline,
        SnapshotFact,
    },
};
use nocterm_core::{
    SharedCause,
    allocation::{Allocated, AllocationLease},
};
use nocterm_hub_model::cloud::events::{CloudEvent, CloudSnapshotPayload, MAX_SNAPSHOT_EVENTS};
use nocterm_keys::cloud::{OpenedRecord, open_event, open_snapshot};
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum ObservationError {
    #[error(transparent)]
    Material(#[from] AccountMaterialError),
    #[error(transparent)]
    Projection(#[from] ProjectionError),
    #[error("the original runner observation failed authentication or canonical decoding")]
    Authentication(#[source] SharedCause),
}
fn authentication(error: impl std::error::Error + Send + Sync + 'static) -> ObservationError {
    ObservationError::Authentication(SharedCause::new(error))
}

/// A retained native material handle cannot restore a replaced account login.
/// A caller must independently establish fresh R heads before bootstrap/replay.
/// The captured binding is that authenticated CURRENT upper writer, shared by
/// all older original record writers; a future writer requires a fresh rebind.
pub struct AccountObservationDecoder {
    material: Arc<AccountChatMaterial>,
    binding: ProjectionBinding,
}
impl AccountObservationDecoder {
    pub fn new(
        material: Arc<AccountChatMaterial>,
        binding: ProjectionBinding,
    ) -> Result<Self, ObservationError> {
        material.check_current()?;
        if binding.stream() != &material.request().stream {
            return Err(ProjectionError::WrongSource.into());
        }
        Ok(Self { material, binding })
    }
    /// SDK scratch only; native JSON/content/container copies are sub-admitted
    /// separately from the remaining supplied actual reservation while decoding.
    /// Original caller ciphertext and transport/runtime baseline are not included.
    pub fn work_bytes(ciphertext_bytes: usize, snapshot: bool) -> Result<usize, ObservationError> {
        work::scratch(
            ciphertext_bytes,
            if snapshot { MAX_SNAPSHOT_EVENTS } else { 1 },
        )
        .ok_or(ProjectionError::Capacity.into())
    }
    pub fn event(
        &self,
        anchor: &HeadAnchor,
        before: ProjectionCursor,
        sealed: &[u8],
        allocation: AllocationLease,
    ) -> Result<Allocated<ProjectionEvent>, ObservationError> {
        if anchor.kind() != HeadAnchorKind::Event
            || before
                .sequence()
                .advance(1)
                .map_err(|_| ProjectionError::Capacity)?
                != anchor.position().event()
        {
            return Err(ProjectionError::Discontinuous.into());
        }
        let mut budget = budget::DecodeBudget::with_scratch(
            &allocation,
            Self::work_bytes(sealed.len(), false)?,
        )?;
        budget.admit(8192)?;
        let opened = self.open(anchor, sealed)?;
        let record = opened.record().map_err(authentication)?;
        let event = CloudEvent::from_record(&record).map_err(authentication)?;
        let value = ProjectionEvent {
            binding: self.binding.clone(),
            id: anchor.id(),
            before,
            head: anchor.ciphertext(),
            control: ChatControlRevision::from_u64(event.control_revision),
            time: Timestamp::from_millis(event.timestamp_ms),
            event: mapping::event(&self.material.request().stream, &event.payload, &mut budget)?,
        };
        self.material.check_current()?;
        Ok(Allocated::new(value, allocation))
    }
    /// Sparse snapshot facts retain only their signed time/control. The outer
    /// real anchor supplies the baseline cursor; no inner event IDs are invented.
    pub fn snapshot(
        &self,
        anchor: &HeadAnchor,
        sealed: &[u8],
        allocation: AllocationLease,
    ) -> Result<Allocated<SnapshotBaseline>, ObservationError> {
        if anchor.kind() != HeadAnchorKind::Snapshot {
            return Err(ProjectionError::InvalidRecord.into());
        }
        let mut budget =
            budget::DecodeBudget::with_scratch(&allocation, Self::work_bytes(sealed.len(), true)?)?;
        budget.admit(8192)?;
        let opened = self.open(anchor, sealed)?;
        let record = opened.record().map_err(authentication)?;
        let snapshot = CloudSnapshotPayload::from_record(&record).map_err(authentication)?;
        if snapshot.events.len() > nocterm_cloud_model::projection::MAX_PROJECTION_ENTRIES {
            return Err(ProjectionError::Capacity.into());
        }
        budget.admit(
            snapshot
                .events
                .len()
                .checked_mul(std::mem::size_of::<SnapshotFact>())
                .ok_or(ProjectionError::Capacity)?,
        )?;
        let mut facts = Vec::with_capacity(snapshot.events.len());
        for event in &snapshot.events {
            facts.push(SnapshotFact {
                control: ChatControlRevision::from_u64(event.control_revision),
                time: Timestamp::from_millis(event.timestamp_ms),
                event: mapping::event(
                    &self.material.request().stream,
                    &event.payload,
                    &mut budget,
                )?,
            });
        }
        let baseline = SnapshotBaseline::new(
            self.binding.clone(),
            anchor.clone(),
            ChatControlRevision::from_u64(snapshot.control_revision),
            Timestamp::from_millis(snapshot.timestamp_ms),
            snapshot.omitted_earlier,
            facts,
        )?;
        self.material.check_current()?;
        Ok(Allocated::new(baseline, allocation))
    }
    fn open(&self, anchor: &HeadAnchor, sealed: &[u8]) -> Result<OpenedRecord, ObservationError> {
        self.material.check_current()?;
        if anchor.position().writer() > self.binding.writer() {
            return Err(ProjectionError::WrongSource.into());
        }
        if &Sha256::digest(sealed)[..] != anchor.ciphertext().as_bytes() {
            return Err(ProjectionError::InvalidRecord.into());
        }
        let request = self.material.request();
        let context =
            record_context(&request.stream, request.epoch, anchor).map_err(authentication)?;
        let opened = self.material.with_key(|key| {
            let result = match anchor.kind() {
                HeadAnchorKind::Event => open_event(key, &request.pins.signing, &context, sealed),
                HeadAnchorKind::Snapshot => {
                    open_snapshot(key, &request.pins.signing, &context, sealed)
                }
            };
            result.map_err(|error| AccountMaterialError::Backend(SharedCause::new(error)))
        })?;
        self.material.check_current()?;
        Ok(opened)
    }
}

#[cfg(test)]
pub(super) fn decode_opened_event(
    stream: &nocterm_cloud_model::authority::StreamBinding,
    opened: &OpenedRecord,
    allocation: &AllocationLease,
) -> Result<nocterm_cloud_model::coordinator::CoordinatorEvent, ObservationError> {
    let record = opened.record().map_err(authentication)?;
    let event = CloudEvent::from_record(&record).map_err(authentication)?;
    let mut budget = budget::DecodeBudget::new(allocation)?;
    Ok(mapping::event(stream, &event.payload, &mut budget)?)
}
