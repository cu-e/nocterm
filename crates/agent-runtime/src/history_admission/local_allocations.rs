//! Local immutable generations follow the actual Arc through the existing writer.
use super::{
    HistoryAdmissionError, HistoryAdmissionHandle, HistoryBytesPermit, HistoryPayloadPermit,
    HistoryRegistration, Requester,
};
use gpui::{App, EntityId};
use nocterm_ai::{history::SharedChat, storage::AgentStorageError};
use std::sync::{Arc, Weak};

pub(super) struct TrackedSnapshot {
    value: Weak<SharedChat>,
    _bytes: HistoryBytesPermit,
    _payload: HistoryPayloadPermit,
}
impl TrackedSnapshot {
    pub(super) fn alive(&self) -> bool {
        self.value.strong_count() > 0
    }
}
/// Captured before any Runtime/Thread mutable borrow. It performs no service lookup.
#[derive(Clone)]
pub struct LocalHistoryAllocations {
    admission: HistoryAdmissionHandle,
    runtime: EntityId,
}
pub struct LocalSnapshotAllocation {
    admission: HistoryAdmissionHandle,
    bytes: HistoryBytesPermit,
    payload: HistoryPayloadPermit,
}
impl LocalHistoryAllocations {
    pub(crate) fn new(admission: HistoryAdmissionHandle, runtime: EntityId) -> Self {
        Self { admission, runtime }
    }
    pub(crate) fn handle(&self) -> HistoryAdmissionHandle {
        self.admission.clone()
    }
    pub fn history_admission(&self) -> HistoryAdmissionHandle {
        self.admission.clone()
    }
    pub(crate) fn retain_external(
        &self,
        requester: EntityId,
        existing: usize,
        value: Arc<SharedChat>,
        cx: &mut App,
    ) -> Result<Arc<SharedChat>, AgentStorageError> {
        let tracked = self
            .admission
            .0
            .state
            .borrow()
            .snapshots
            .iter()
            .any(|entry| entry.value.ptr_eq(&Arc::downgrade(&value)));
        if tracked {
            return Ok(value);
        }
        let capacity = value.retained_bytes()?;
        self.reserve_snapshot(requester, existing, capacity, cx)
            .map_err(HistoryAdmissionError::into_storage)?
            .retain(value)
    }
    pub(crate) fn transfer_prepared(
        &self,
        source: &HistoryRegistration,
        remaining: usize,
        bytes: usize,
        payload: HistoryPayloadPermit,
        cx: &mut App,
    ) -> Result<nocterm_core::allocation::AllocationLease, AgentStorageError> {
        self.admission
            .require_payload(&payload)
            .map_err(HistoryAdmissionError::into_storage)?;
        let proof = self
            .admission
            .reserve_observed_buffer(source, vec![self.runtime], remaining, bytes, cx)
            .map_err(HistoryAdmissionError::into_storage)?;
        Ok(nocterm_core::allocation::AllocationLease::new(
            bytes,
            1,
            (proof, payload),
        ))
    }
    pub fn reserve_resident(
        &self,
        requester: Option<EntityId>,
        cx: &mut App,
    ) -> Result<HistoryPayloadPermit, HistoryAdmissionError> {
        let mut excluded = vec![self.runtime];
        excluded.extend(requester);
        self.admission.reserve_payload_excluding(&excluded, cx)
    }
    /// Snapshot metadata/rows are measured while borrowed, before cloning them.
    pub fn reserve_snapshot(
        &self,
        requester: EntityId,
        existing: usize,
        capacity: usize,
        cx: &mut App,
    ) -> Result<LocalSnapshotAllocation, HistoryAdmissionError> {
        let excluded = vec![self.runtime, requester];
        let payload = self.admission.reserve_payload_excluding(&excluded, cx)?;
        self.admission.decide(
            Requester::Entities(excluded),
            existing.saturating_add(capacity),
            cx,
        )?;
        let bytes = self.admission.charge(capacity)?;
        Ok(LocalSnapshotAllocation {
            admission: self.admission.clone(),
            bytes,
            payload,
        })
    }
}
impl LocalSnapshotAllocation {
    /// The actual decoded body generation moves to its resident owner with this
    /// same slot. Bytes remain with the worker/result until atomic publication.
    pub fn decode_allocation(
        self,
    ) -> (
        nocterm_core::allocation::AllocationLease,
        HistoryPayloadPermit,
    ) {
        let slot = self.payload.clone();
        (self.into_lease(), slot)
    }

    /// Transfer an already reserved standalone preparation into its actual owner.
    pub fn into_lease(self) -> nocterm_core::allocation::AllocationLease {
        nocterm_core::allocation::AllocationLease::new(
            self.bytes.bytes(),
            1,
            (self.bytes, self.payload),
        )
    }

    /// Track the actual SharedChat allocation, including active/latest generations,
    /// shutdown capture, and a detached native worker after its GUI owner drops.
    /// Arc clones/retries keep this one proof, not one proof per observer.
    pub fn retain(self, value: Arc<SharedChat>) -> Result<Arc<SharedChat>, AgentStorageError> {
        let actual = value.retained_bytes()?;
        if actual > self.bytes.bytes() {
            return Err(AgentStorageError::ResidentCapacity {
                retained: 0,
                incoming: actual,
                limit: self.bytes.bytes(),
            });
        }
        let bytes = self
            .admission
            .shrink_buffer(self.bytes, actual)
            .map_err(HistoryAdmissionError::into_storage)?;
        let mut state = self.admission.0.state.borrow_mut();
        state.snapshots.retain(TrackedSnapshot::alive);
        if !state
            .snapshots
            .iter()
            .any(|entry| entry.value.ptr_eq(&Arc::downgrade(&value)))
        {
            state.snapshots.push(TrackedSnapshot {
                value: Arc::downgrade(&value),
                _bytes: bytes,
                _payload: self.payload,
            });
        }
        Ok(value)
    }
}
