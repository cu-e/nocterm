//! Independently persisted transport identity and charged immutable publications.
use super::*;
use crate::{
    authority::StreamBinding,
    coordinator::{CommandReceiveBudget, IntakeLane, IntakePoolLimits},
    identity::*,
    payload::OpaqueBytes,
};
use nocterm_core::allocation::{Allocated, AllocationLease};
use std::sync::Arc;

/// Transport deduplication identity. It is never the signed EventId.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct OutboxDeliveryId([u8; 16]);
impl OutboxDeliveryId {
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}
#[derive(Debug)]
pub struct OutboxPublication {
    stream: StreamBinding,
    entry: OutboxEntry,
}
/// Only sharing this carrier shares a proof; cloning an entry alone does not.
#[derive(Clone)]
pub struct ResidentPublication(Arc<Allocated<OutboxPublication>>);
impl std::fmt::Debug for ResidentPublication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.entry().fmt(f)
    }
}
impl ResidentPublication {
    pub fn new(
        stream: StreamBinding,
        entry: OutboxEntry,
        allocation: AllocationLease,
    ) -> Result<Self, JournalError> {
        if allocation.bytes() < Self::required_bytes(entry.sealed().len())?
            || allocation.payloads() < 1
            || entry
                .anchor()
                .run()
                .is_some_and(|run| run.stream() != stream)
        {
            return Err(JournalError::Capacity);
        }
        Ok(Self(Arc::new(Allocated::new(
            OutboxPublication { stream, entry },
            allocation,
        ))))
    }
    /// Original ciphertext plus its native owned envelope payload, serialized
    /// plaintext and sealed native envelope; metadata <=16KiB, AEAD overhead <=512B.
    /// Public encoder scratch is reserved separately by that actual owner.
    pub fn required_bytes(sealed: usize) -> Result<usize, JournalError> {
        let payload_clone = sealed;
        let plaintext = sealed
            .checked_add(16 * 1024)
            .ok_or(JournalError::Capacity)?;
        let ciphertext = plaintext.checked_add(512).ok_or(JournalError::Capacity)?;
        [sealed, payload_clone, plaintext, ciphertext]
            .into_iter()
            .try_fold(0usize, |n, b| {
                n.checked_add(b).ok_or(JournalError::Capacity)
            })
    }
    pub fn stream(&self) -> &StreamBinding {
        &self.0.value().stream
    }
    pub fn entry(&self) -> &OutboxEntry {
        &self.0.value().entry
    }
    pub fn allocation(&self) -> &AllocationLease {
        self.0.allocation()
    }
    pub fn anchor(&self) -> &HeadAnchor {
        self.entry().anchor()
    }
    pub fn key_epoch(&self) -> ChatKeyEpoch {
        self.entry().key_epoch()
    }
    pub fn delivery_id(&self) -> OutboxDeliveryId {
        self.entry().delivery_id()
    }
    pub fn id(&self) -> EventId {
        self.entry().id()
    }
    pub fn sequence(&self) -> EventSequence {
        self.entry().sequence()
    }
    pub fn record(&self) -> JournalSequence {
        self.entry().record()
    }
    pub fn sealed(&self) -> &OpaqueBytes {
        self.entry().sealed()
    }
}
#[derive(Clone, Copy, Debug)]
pub struct PublicationAllocation {
    bytes: usize,
    payloads: usize,
}
impl PublicationAllocation {
    pub fn new(bytes: usize, payloads: usize) -> Result<Self, JournalError> {
        if bytes == 0 || payloads == 0 {
            return Err(JournalError::Capacity);
        }
        Ok(Self { bytes, payloads })
    }
    pub fn bytes(self) -> usize {
        self.bytes
    }
    pub fn payloads(self) -> usize {
        self.payloads
    }
}
pub trait PublicationWorkBudget: Send + Sync {
    fn try_reserve(&self, request: PublicationAllocation) -> Result<AllocationLease, JournalError>;
}
/// Explicit finite class. Ordinary work shares actual command capacity;
/// maintenance uses a separate finite pool of the same generation implementation.
#[derive(Clone)]
pub struct PublicationBudget {
    owner: PublicationOwner,
}
#[derive(Clone)]
enum PublicationOwner {
    Intake(CommandReceiveBudget, IntakeLane),
    Maintenance(Arc<crate::coordinator::intake::pool::Pool>),
}
impl PublicationBudget {
    pub fn ordinary(owner: CommandReceiveBudget) -> Self {
        Self {
            owner: PublicationOwner::Intake(owner, IntakeLane::Ordinary),
        }
    }
    pub fn urgent(owner: CommandReceiveBudget) -> Self {
        Self {
            owner: PublicationOwner::Intake(owner, IntakeLane::Urgent),
        }
    }
    pub fn maintenance(limits: IntakePoolLimits) -> Self {
        Self {
            owner: PublicationOwner::Maintenance(crate::coordinator::intake::pool::Pool::new(
                limits,
            )),
        }
    }
}
impl PublicationWorkBudget for PublicationBudget {
    fn try_reserve(&self, request: PublicationAllocation) -> Result<AllocationLease, JournalError> {
        match &self.owner {
            PublicationOwner::Intake(owner, lane) => {
                owner.try_reserve(*lane, request.bytes, request.payloads)
            }
            PublicationOwner::Maintenance(owner) => owner.reserve(request.bytes, request.payloads),
        }
        .map_err(|_| JournalError::Capacity)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeliveryCursor(u64);
impl DeliveryCursor {
    pub fn new(value: u64) -> Result<Self, JournalError> {
        if value == 0 {
            Err(JournalError::InvalidBatch)
        } else {
            Ok(Self(value))
        }
    }
    pub fn get(self) -> u64 {
        self.0
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboxStored {
    stream: StreamBinding,
    delivery_id: OutboxDeliveryId,
    ciphertext: Fingerprint,
    cursor: DeliveryCursor,
}
impl OutboxStored {
    pub fn new(
        stream: StreamBinding,
        delivery_id: OutboxDeliveryId,
        ciphertext: Fingerprint,
        cursor: DeliveryCursor,
    ) -> Self {
        Self {
            stream,
            delivery_id,
            ciphertext,
            cursor,
        }
    }
    pub fn stream(&self) -> &StreamBinding {
        &self.stream
    }
    pub fn delivery_id(&self) -> OutboxDeliveryId {
        self.delivery_id
    }
    pub fn ciphertext(&self) -> &Fingerprint {
        &self.ciphertext
    }
    pub fn cursor(&self) -> DeliveryCursor {
        self.cursor
    }
}
pub trait OutboxPublicationReader: Send + Sync {
    fn next(
        &self,
        stream: StreamBinding,
        after: EventSequence,
        limits: JournalReadLimits,
        work: Arc<dyn PublicationWorkBudget>,
    ) -> JournalFuture<Option<ResidentPublication>>;
}
/// Captures the configured original account/R-source and full trust pins before
/// any await. Rechecks that same captured authority immediately before starting
/// each RPC and after awaited I/O, before publishing its result. Native callers
/// also recheck after native reads; Stop/tombstone under a still-live original
/// source permits fact-only immutable upload, source expiry/revocation does not.
pub trait OutboxPublicationTransport: Send + Sync {
    fn publish(&self, publication: ResidentPublication) -> JournalFuture<OutboxStored>;
    fn purge(&self, candidate: SnapshotCandidate) -> JournalFuture<SnapshotPurged>;
}
/// Delivery receipts only advance a FIFO transport watermark, never command C.
pub trait OutboxDeliveryJournal: Send + Sync {
    fn record_delivery(
        &self,
        current: CleanupLease,
        receipt: OutboxStored,
    ) -> JournalFuture<EventSequence>;
    fn delivered(&self, stream: StreamBinding) -> JournalFuture<EventSequence>;
}
