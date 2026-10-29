//! One Local resident body and separately retained immutable snapshot generations.
use gpui::{App, EntityId};
use nocterm_agent_runtime::history_admission::{
    HistoryPayloadPermit, LocalHistoryAllocations, LocalSnapshotAllocation,
};
use nocterm_ai::storage::AgentStorageError;

pub(crate) type LocalPayloadAllocation = (LocalHistoryAllocations, HistoryPayloadPermit);

pub(super) struct LocalPayloads {
    allocations: LocalHistoryAllocations,
    resident: Option<HistoryPayloadPermit>,
    requester: EntityId,
}
impl LocalPayloads {
    pub(super) fn new(
        allocations: LocalHistoryAllocations,
        resident: HistoryPayloadPermit,
        requester: EntityId,
    ) -> Self {
        Self {
            allocations,
            resident: Some(resident),
            requester,
        }
    }
    pub(super) fn order(&self) -> Option<u64> {
        self.resident.as_ref().map(HistoryPayloadPermit::order)
    }
    pub(super) fn archive(&mut self) {
        self.resident.take();
    }
    pub(super) fn ensure_resident(&mut self, cx: &mut App) -> Result<(), AgentStorageError> {
        if self.resident.is_none() {
            self.resident = Some(
                self.allocations
                    .reserve_resident(Some(self.requester), cx)
                    .map_err(|error| error.into_storage())?,
            );
        }
        Ok(())
    }
    pub(super) fn adopt_resident(&mut self, payload: HistoryPayloadPermit) {
        self.resident = Some(payload);
    }
    pub(super) fn history_admission(
        &self,
    ) -> nocterm_agent_runtime::history_admission::HistoryAdmissionHandle {
        self.allocations.history_admission()
    }
    pub(super) fn restart(
        &self,
        document: &super::document::ChatDocument,
        input: &super::document::Inputs<'_>,
        existing: usize,
        cx: &mut App,
    ) -> Result<nocterm_core::allocation::AllocationLease, AgentStorageError> {
        Ok(self
            .snapshot(existing, document.restart_capacity(input)?, cx)?
            .into_lease())
    }
    pub(super) fn fork(
        &self,
        document: &super::document::ChatDocument,
        input: &super::document::Inputs<'_>,
        existing: usize,
        len: usize,
        cx: &mut App,
    ) -> Result<nocterm_core::allocation::AllocationLease, AgentStorageError> {
        Ok(self
            .snapshot(existing, document.fork_capacity(input, len)?, cx)?
            .into_lease())
    }
    pub(super) fn snapshot(
        &self,
        existing: usize,
        capacity: usize,
        cx: &mut App,
    ) -> Result<LocalSnapshotAllocation, AgentStorageError> {
        self.allocations
            .reserve_snapshot(self.requester, existing, capacity, cx)
            .map_err(|error| error.into_storage())
    }
}
