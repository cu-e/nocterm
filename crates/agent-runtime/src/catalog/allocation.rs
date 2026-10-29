//! Pending scans and retained metadata share the installation's byte allocation.
use crate::history_admission::{
    HistoryAdmissionError, HistoryAdmissionHandle, HistoryBytesPermit, HistoryPayloadPermit,
};
use nocterm_ai::{
    history::{ChatSummary, HistoryCodecError, MAX_RESTORED},
    storage::AgentStorageError,
};

pub(super) const MAX_ROW_BYTES: usize = 32 * 1024;
// At most200 returned rows,200 prepared metadata rows and one native scan
// candidate coexist. OneMiB also covers vector reallocation and assembly state.
pub(super) const WORK_BYTES: usize = (2 * MAX_RESTORED + 1) * MAX_ROW_BYTES + 1024 * 1024;

pub(crate) struct CatalogAllocation {
    slot: HistoryPayloadPermit,
    bytes: HistoryBytesPermit,
}
impl CatalogAllocation {
    pub(super) fn new(slot: HistoryPayloadPermit, bytes: HistoryBytesPermit) -> Self {
        Self { slot, bytes }
    }
    pub(super) fn order(&self) -> u64 {
        self.slot.order()
    }
    pub(super) fn finish(
        self,
        admission: &HistoryAdmissionHandle,
        rows: &[ChatSummary],
        capacity: usize,
    ) -> Result<Self, HistoryAdmissionError> {
        let retained = rows.iter().fold(
            4096usize.saturating_add(capacity.saturating_mul(std::mem::size_of::<ChatSummary>())),
            |total, row| total.saturating_add(row_bytes(row)),
        );
        Ok(Self {
            slot: self.slot,
            bytes: admission.shrink_buffer(self.bytes, retained)?,
        })
    }
    pub(super) fn check(
        rows: Result<Vec<ChatSummary>, AgentStorageError>,
    ) -> Result<Vec<ChatSummary>, AgentStorageError> {
        let rows = rows?;
        if rows.len() > MAX_RESTORED {
            return Err(HistoryCodecError::CollectionLimit.into());
        }
        if rows.iter().any(|row| row_bytes(row) > MAX_ROW_BYTES) {
            return Err(HistoryCodecError::StructuralLimit.into());
        }
        Ok(rows)
    }
}
fn row_bytes(row: &ChatSummary) -> usize {
    // Fixed storage covers inline IDs/epochs/fingerprints and the captured
    // environment's bounded namespace strings (2*128B or a host<=253B).
    // Actual string capacity counts the owned allocations, including decoder
    // spare capacity. Serializing fixed fingerprint bytes as JSON nodes would
    // overstate this metadata's memory rather than measure its retained owners.
    4096usize
        .saturating_add(row.id.capacity())
        .saturating_add(row.agent_id.capacity())
        .saturating_add(row.title.as_ref().map_or(0, String::capacity))
        .saturating_add(row.name.as_ref().map_or(0, String::capacity))
        .saturating_add(row.model.as_ref().map_or(0, String::capacity))
}
