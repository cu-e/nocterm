use nocterm_cloud_model::{
    journal::{JournalBatch, JournalError},
    payload::MAX_LOGICAL_PAYLOAD_BYTES,
};
use serde::{Deserialize, Serialize};

/// Authenticated logical storage charges. Replaced derived objects are released;
/// immutable command/effect records remain charged until authorized compaction.
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StorageBudget {
    total: u64,
    checkpoint: u64,
    snapshot: u64,
    outbox: u64,
    pending_snapshot: u64,
    public_snapshot: u64,
}
impl StorageBudget {
    pub(crate) fn total(self) -> u64 {
        self.total
    }
    pub(crate) fn validate(self) -> Result<(), JournalError> {
        let subtotal = self
            .checkpoint
            .checked_add(self.snapshot)
            .and_then(|value| value.checked_add(self.outbox))
            .and_then(|value| value.checked_add(self.pending_snapshot))
            .and_then(|value| value.checked_add(self.public_snapshot))
            .ok_or(JournalError::Corrupt)?;
        if subtotal > self.total
            || self.total > 16 * 1024 * 1024 * 1024
            || self.checkpoint > MAX_LOGICAL_PAYLOAD_BYTES as u64
            || self.snapshot > MAX_LOGICAL_PAYLOAD_BYTES as u64 + 16 * 1024
        {
            return Err(JournalError::Corrupt);
        }
        Ok(())
    }
    pub(crate) fn appended(self, batch: &JournalBatch) -> Result<Self, JournalError> {
        let checkpoint = batch
            .checkpoint()
            .map(|checkpoint| checkpoint.bytes().len() as u64);
        let released = if checkpoint.is_some() {
            self.checkpoint
        } else {
            0
        };
        let total = self
            .total
            .checked_sub(released)
            .and_then(|value| value.checked_add(batch.budget_bytes() as u64))
            .ok_or(JournalError::Capacity)?;
        let incoming_outbox = batch.outbox().iter().try_fold(0u64, |total, entry| {
            total
                .checked_add(entry.sealed().len() as u64)
                .ok_or(JournalError::Capacity)
        })?;
        let next = Self {
            total,
            checkpoint: checkpoint.unwrap_or(self.checkpoint),
            snapshot: self.snapshot,
            pending_snapshot: self.pending_snapshot,
            public_snapshot: self.public_snapshot,
            outbox: self
                .outbox
                .checked_add(incoming_outbox)
                .ok_or(JournalError::Capacity)?,
        };
        next.validate()?;
        Ok(next)
    }
    pub(crate) fn maintenance(self, archive: usize, public: usize) -> Result<Self, JournalError> {
        if self.pending_snapshot != 0 {
            return Err(JournalError::Conflict);
        }
        let pending = (archive as u64)
            .checked_add(public as u64)
            .and_then(|n| n.checked_add(64 * 1024))
            .ok_or(JournalError::Capacity)?;
        let mut next = self;
        next.pending_snapshot = pending;
        next.total = next
            .total
            .checked_add(pending)
            .ok_or(JournalError::Capacity)?;
        next.validate()?;
        Ok(next)
    }
    pub(crate) fn activated(
        self,
        records: u64,
        outbox: u64,
        archive: usize,
        public: usize,
    ) -> Result<Self, JournalError> {
        let released = [
            records,
            outbox,
            self.pending_snapshot,
            self.public_snapshot,
            self.snapshot,
        ]
        .into_iter()
        .try_fold(0u64, |n, b| n.checked_add(b).ok_or(JournalError::Corrupt))?;
        let snapshot = (archive as u64)
            .checked_add(16 * 1024)
            .ok_or(JournalError::Capacity)?;
        let public_snapshot = (public as u64)
            .checked_add(16 * 1024)
            .ok_or(JournalError::Capacity)?;
        let total = self
            .total
            .checked_sub(released)
            .and_then(|n| n.checked_add(snapshot))
            .and_then(|n| n.checked_add(public_snapshot))
            .ok_or(JournalError::Corrupt)?;
        let next = Self {
            total,
            checkpoint: self.checkpoint,
            snapshot,
            outbox: self
                .outbox
                .checked_sub(outbox)
                .ok_or(JournalError::Corrupt)?,
            pending_snapshot: 0,
            public_snapshot,
        };
        next.validate()?;
        Ok(next)
    }
}
