//! Atomic staging and activation; transport never runs inside this native owner.
mod capture;
mod codec;
mod deliveries;
pub(super) mod storage;
mod transition;
use super::{Driver, DurableRunJournal, head, integer, storage as sql_error};
use nocterm_cloud_model::{authority::StreamBinding, identity::*, journal::*};
use rusqlite::{Connection, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Injected maintenance capability. Its caller polls native work through the
/// coordinator's common tracked JournalIo gate, never a network task's executor.
#[derive(Clone)]
pub struct DurableSnapshotPublication {
    journal: DurableRunJournal,
    encoder: Arc<dyn CoordinatorSnapshotEncoder>,
    work: Arc<dyn PublicationWorkBudget>,
    limits: SnapshotLimits,
}
impl DurableSnapshotPublication {
    pub fn new(
        journal: DurableRunJournal,
        encoder: Arc<dyn CoordinatorSnapshotEncoder>,
        work: Arc<dyn PublicationWorkBudget>,
        limits: SnapshotLimits,
    ) -> Self {
        Self {
            journal,
            encoder,
            work,
            limits,
        }
    }
}
impl JournalSnapshotPublication for DurableSnapshotPublication {
    fn prepare(&self, current: CleanupLease) -> JournalFuture<SnapshotCandidate> {
        let owner = self.clone();
        Box::pin(async move {
            owner
                .journal
                .with_stream(current.writer().stream(), |driver| {
                    capture::prepare(
                        driver,
                        &current,
                        owner.encoder.as_ref(),
                        owner.work.as_ref(),
                        owner.limits,
                    )
                })
        })
    }
    fn pending(&self, stream: StreamBinding) -> JournalFuture<Option<SnapshotCandidate>> {
        let owner = self.clone();
        Box::pin(async move {
            owner.journal.with_stream(&stream, |driver| {
                let _metadata = owner
                    .work
                    .try_reserve(PublicationAllocation::new(12 * 1024 * 1024, 1)?)?;
                let db = driver.connection.transaction().map_err(sql_error)?;
                super::metadata::snapshot_rows(&db, driver.limits.chats)?;
                let head = head::required(&db, driver.cipher.as_ref(), &stream)?;
                storage::load(&db, driver.cipher.as_ref(), &head)?
                    .pending
                    .map(codec::Candidate::restore)
                    .transpose()
            })
        })
    }
    fn record_stored(
        &self,
        current: CleanupLease,
        receipt: OutboxStored,
    ) -> JournalFuture<SnapshotCandidate> {
        let owner = self.clone();
        Box::pin(async move {
            owner
                .journal
                .with_stream(current.writer().stream(), |driver| {
                    let _metadata = owner
                        .work
                        .try_reserve(PublicationAllocation::new(12 * 1024 * 1024, 1)?)?;
                    transition::record_stored(driver, &current, &receipt)
                })
        })
    }
    fn activate(
        &self,
        current: CleanupLease,
        purged: SnapshotPurged,
    ) -> JournalFuture<ResidentSnapshot> {
        let owner = self.clone();
        Box::pin(async move {
            owner
                .journal
                .with_stream(current.writer().stream(), |driver| {
                    transition::activate(driver, &current, &purged, owner.work.as_ref())
                })
        })
    }
}
impl OutboxPublicationReader for DurableSnapshotPublication {
    fn next(
        &self,
        stream: StreamBinding,
        after: EventSequence,
        limits: JournalReadLimits,
        work: Arc<dyn PublicationWorkBudget>,
    ) -> JournalFuture<Option<ResidentPublication>> {
        let owner = self.clone();
        Box::pin(async move {
            owner.journal.with_stream(&stream, |driver| {
                let _metadata = owner
                    .work
                    .try_reserve(PublicationAllocation::new(12 * 1024 * 1024, 1)?)?;
                let db = driver.connection.transaction().map_err(sql_error)?;
                let head = head::required(&db, driver.cipher.as_ref(), &stream)?;
                if after == head.position().events() {
                    return Ok(None);
                }
                let next = after.advance(1)?;
                let length = storage::outbox_length(&db, &stream, next)?;
                if length > limits.bytes() {
                    return Err(JournalError::Capacity);
                }
                let allocation = storage::reserve_decode(work.as_ref(), length)?;
                let entry = super::replay::outbox_from(
                    &db,
                    driver.cipher.as_ref(),
                    &stream,
                    &head,
                    after,
                    JournalReadLimits::new(1, limits.bytes())?,
                )?
                .pop()
                .ok_or(JournalError::Corrupt)?;
                if entry.anchor().ciphertext().as_bytes().as_slice()
                    != Sha256::digest(entry.sealed().as_bytes()).as_slice()
                {
                    return Err(JournalError::Corrupt);
                }
                ResidentPublication::new(stream.clone(), entry, allocation).map(Some)
            })
        })
    }
}

impl OutboxDeliveryJournal for DurableSnapshotPublication {
    fn record_delivery(
        &self,
        current: CleanupLease,
        receipt: OutboxStored,
    ) -> JournalFuture<EventSequence> {
        let owner = self.clone();
        Box::pin(async move {
            owner
                .journal
                .with_stream(current.writer().stream(), |driver| {
                    let _metadata = owner
                        .work
                        .try_reserve(PublicationAllocation::new(12 * 1024 * 1024, 1)?)?;
                    deliveries::record(driver, &current, &receipt, owner.work.as_ref())
                })
        })
    }
    fn delivered(&self, stream: StreamBinding) -> JournalFuture<EventSequence> {
        let owner = self.clone();
        Box::pin(async move {
            owner.journal.with_stream(&stream, |driver| {
                Ok(
                    head::required(&driver.connection, driver.cipher.as_ref(), &stream)?
                        .acknowledged(),
                )
            })
        })
    }
}
