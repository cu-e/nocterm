//! Signed public baseline transfer and independent native recovery prefix.
use super::*;
use crate::{authority::StreamBinding, identity::*};

#[derive(Clone, Copy, Debug)]
pub struct SnapshotLimits {
    records: usize,
    bytes: usize,
    output: usize,
}
impl SnapshotLimits {
    pub fn new(records: usize, bytes: usize, output: usize) -> Result<Self, JournalError> {
        if records == 0
            || records > 4096
            || bytes == 0
            || bytes > 128 * 1024 * 1024
            || output == 0
            || output > 16 * 1024 * 1024
        {
            return Err(JournalError::Capacity);
        }
        Ok(Self {
            records,
            bytes,
            output,
        })
    }
    pub fn records(self) -> usize {
        self.records
    }
    pub fn bytes(self) -> usize {
        self.bytes
    }
    pub fn output(self) -> usize {
        self.output
    }
}
/// Actual signed public records, never inferred from native record bytes.
pub struct SnapshotProjectionSource {
    stream: StreamBinding,
    material: ChatKeyEpoch,
    covered: JournalPosition,
    previous: Option<ResidentPublication>,
    records: Vec<ResidentPublication>,
}
impl SnapshotProjectionSource {
    pub fn new(
        stream: StreamBinding,
        material: ChatKeyEpoch,
        covered: JournalPosition,
        previous: Option<ResidentPublication>,
        records: Vec<ResidentPublication>,
        limits: SnapshotLimits,
    ) -> Result<Self, JournalError> {
        if records.len() > limits.records {
            return Err(JournalError::Capacity);
        }
        let mut after = EventSequence::default();
        let mut bytes = 0usize;
        if let Some(previous) = &previous {
            if previous.anchor().kind() != HeadAnchorKind::Snapshot
                || previous.stream() != &stream
                || previous.key_epoch() != material
            {
                return Err(JournalError::Corrupt);
            }
            after = previous.sequence();
            bytes = previous.sealed().len();
        }
        for record in &records {
            bytes = bytes
                .checked_add(record.sealed().len())
                .ok_or(JournalError::Capacity)?;
            if record.stream() != &stream
                || record.key_epoch() != material
                || record.sequence() != after.advance(1)?
                || record.record() > covered.records()
            {
                return Err(JournalError::Corrupt);
            }
            after = record.sequence();
        }
        if after != covered.events() {
            return Err(JournalError::EventGap {
                expected: covered.events(),
                actual: after,
            });
        }
        if bytes > limits.bytes {
            return Err(JournalError::Capacity);
        }
        Ok(Self {
            stream,
            material,
            covered,
            previous,
            records,
        })
    }
    pub fn stream(&self) -> &StreamBinding {
        &self.stream
    }
    pub fn material(&self) -> ChatKeyEpoch {
        self.material
    }
    pub fn covered(&self) -> JournalPosition {
        self.covered
    }
    pub fn previous(&self) -> Option<&ResidentPublication> {
        self.previous.as_ref()
    }
    pub fn records(&self) -> &[ResidentPublication] {
        &self.records
    }
}
pub trait CoordinatorSnapshotEncoder: Send + Sync {
    fn encode(
        &self,
        lease: &WriterLease,
        next: JournalPosition,
        source: &SnapshotProjectionSource,
        limits: SnapshotLimits,
        work: &dyn PublicationWorkBudget,
    ) -> Result<ResidentPublication, JournalError>;
}
/// Bounded descriptive durable metadata; it proves neither crypto nor delivery.
#[derive(Clone, Debug)]
pub struct SnapshotDescriptor {
    pub stream: StreamBinding,
    pub material: ChatKeyEpoch,
    pub anchor: HeadAnchor,
    pub delivery: OutboxDeliveryId,
    pub length: usize,
    pub covered: JournalPosition,
    pub archive: Fingerprint,
    pub archive_length: usize,
}
/// Small durable descriptor. Large native/public bodies remain in bounded storage.
#[derive(Clone, Debug)]
pub struct SnapshotCandidate {
    stream: StreamBinding,
    material: ChatKeyEpoch,
    anchor: HeadAnchor,
    delivery: OutboxDeliveryId,
    length: usize,
    covered: JournalPosition,
    archive: Fingerprint,
    archive_length: usize,
    stored: Option<OutboxStored>,
}
impl SnapshotCandidate {
    pub fn new(
        publication: &ResidentPublication,
        covered: JournalPosition,
        archive: Fingerprint,
        archive_length: usize,
    ) -> Result<Self, JournalError> {
        let position = publication.anchor().position();
        let next = covered.next(1, 1, false)?;
        if publication.anchor().kind() != HeadAnchorKind::Snapshot
            || position.event() != next.events()
            || position.journal() != next.records()
            || position.revision() != next.revision()
            || archive_length == 0
            || archive_length > 16 * 1024 * 1024
        {
            return Err(JournalError::InvalidBatch);
        }
        Ok(Self {
            stream: publication.stream().clone(),
            material: publication.key_epoch(),
            anchor: publication.anchor().clone(),
            delivery: publication.delivery_id(),
            length: publication.sealed().len(),
            covered,
            archive,
            archive_length,
            stored: None,
        })
    }
    pub fn from_durable(
        description: SnapshotDescriptor,
        stored: Option<OutboxStored>,
    ) -> Result<Self, JournalError> {
        let SnapshotDescriptor {
            stream,
            material,
            anchor,
            delivery,
            length,
            covered,
            archive,
            archive_length,
        } = description;
        let next = covered.next(1, 1, false)?;
        if anchor.kind() != HeadAnchorKind::Snapshot
            || anchor.position().event() != next.events()
            || anchor.position().journal() != next.records()
            || anchor.position().revision() != next.revision()
            || length == 0
            || archive_length == 0
            || archive_length > 16 * 1024 * 1024
        {
            return Err(JournalError::Corrupt);
        }
        let mut result = Self {
            stream,
            material,
            anchor,
            delivery,
            length,
            covered,
            archive,
            archive_length,
            stored: None,
        };
        if let Some(stored) = stored {
            result = result.record_stored(stored)?;
        }
        Ok(result)
    }
    pub fn stream(&self) -> &StreamBinding {
        &self.stream
    }
    pub fn material(&self) -> ChatKeyEpoch {
        self.material
    }
    pub fn anchor(&self) -> &HeadAnchor {
        &self.anchor
    }
    pub fn delivery_id(&self) -> OutboxDeliveryId {
        self.delivery
    }
    pub fn length(&self) -> usize {
        self.length
    }
    pub fn covered(&self) -> JournalPosition {
        self.covered
    }
    pub fn archive(&self) -> Fingerprint {
        self.archive
    }
    pub fn archive_length(&self) -> usize {
        self.archive_length
    }
    pub fn stored(&self) -> Option<&OutboxStored> {
        self.stored.as_ref()
    }
    pub fn record_stored(mut self, receipt: OutboxStored) -> Result<Self, JournalError> {
        if receipt.stream() != &self.stream
            || receipt.delivery_id() != self.delivery
            || receipt.ciphertext() != &self.anchor.ciphertext()
            || self.stored.as_ref().is_some_and(|old| old != &receipt)
        {
            return Err(JournalError::Conflict);
        }
        self.stored = Some(receipt);
        Ok(self)
    }
}
#[derive(Clone, Debug)]
pub struct SnapshotPurged {
    stored: OutboxStored,
    through: u64,
}
impl SnapshotPurged {
    pub fn new(stored: OutboxStored, through: u64) -> Result<Self, JournalError> {
        if stored.cursor().get().checked_sub(1) != Some(through) {
            return Err(JournalError::Conflict);
        }
        Ok(Self { stored, through })
    }
    pub fn stored(&self) -> &OutboxStored {
        &self.stored
    }
    pub fn through(&self) -> u64 {
        self.through
    }
}
pub trait JournalSnapshotPublication: Send + Sync {
    fn prepare(&self, current: CleanupLease) -> JournalFuture<SnapshotCandidate>;
    fn pending(&self, stream: StreamBinding) -> JournalFuture<Option<SnapshotCandidate>>;
    fn record_stored(
        &self,
        current: CleanupLease,
        receipt: OutboxStored,
    ) -> JournalFuture<SnapshotCandidate>;
    fn activate(
        &self,
        current: CleanupLease,
        purged: SnapshotPurged,
    ) -> JournalFuture<ResidentSnapshot>;
}
/// Stored native snapshot payload always remains independent of public ciphertext.
pub type ResidentSnapshot = std::sync::Arc<ChargedSnapshot>;
pub struct ChargedSnapshot {
    snapshot: JournalSnapshot,
    allocation: nocterm_core::allocation::AllocationLease,
}
impl ChargedSnapshot {
    pub fn new(
        snapshot: JournalSnapshot,
        allocation: nocterm_core::allocation::AllocationLease,
    ) -> Result<Self, JournalError> {
        if allocation.bytes() < snapshot.payload().len() || allocation.payloads() == 0 {
            return Err(JournalError::Capacity);
        }
        Ok(Self {
            snapshot,
            allocation,
        })
    }
    pub fn stream(&self) -> &StreamBinding {
        self.snapshot.stream()
    }
    pub fn at(&self) -> JournalPosition {
        self.snapshot.at()
    }
    pub fn payload(&self) -> &crate::payload::OpaqueBytes {
        self.snapshot.payload()
    }
    pub fn allocation(&self) -> &nocterm_core::allocation::AllocationLease {
        &self.allocation
    }
    pub fn snapshot(&self) -> &JournalSnapshot {
        &self.snapshot
    }
}
