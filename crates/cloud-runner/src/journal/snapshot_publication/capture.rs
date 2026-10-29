//! Bounded synchronous capture/encode/stage under the caller's one write gate.
use super::*;
use crate::codec::{
    self as native,
    archive::{MAX_ARCHIVE_BYTES, NativeRunArchive},
};
use nocterm_cloud_model::payload::OpaqueBytes;
use nocterm_core::allocation::AllocationLease;

pub(super) fn prepare(
    driver: &mut Driver,
    lease: &CleanupLease,
    encoder: &dyn CoordinatorSnapshotEncoder,
    work: &dyn PublicationWorkBudget,
    limits: SnapshotLimits,
) -> Result<SnapshotCandidate, JournalError> {
    let _metadata = work.try_reserve(PublicationAllocation::new(12 * 1024 * 1024, 1)?)?;
    let cipher = driver.cipher.as_ref();
    let db = driver
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql_error)?;
    let mut catalog = driver.catalog.load(&db, driver.catalog_cipher.as_ref())?;
    driver.catalog.verify_rows(&db, &catalog)?;
    super::super::metadata::snapshot_rows(&db, driver.limits.chats)?;
    let previous = driver.heads.required(&db, lease.writer().stream())?;
    let mut head = head::required(&db, cipher, lease.writer().stream())?;
    head.require_cleanup(lease)?;
    let mut state = storage::load(&db, cipher, &head)?;
    if let Some(candidate) = state.pending {
        return candidate.restore();
    }
    if head.position().records().get() == 0 {
        return Err(JournalError::InvalidBatch);
    }
    let captured = SnapshotCapture::load(&db, cipher, &head, state.active.as_ref(), work, limits)?;
    let stage = StagedSnapshot::encode(captured, lease, encoder, work, limits)?;
    stage.persist(&db, cipher, &mut head, driver.limits.chat_bytes)?;
    state.pending = Some((&stage.candidate).into());
    storage::save(&db, cipher, &mut head, &state)?;
    head.save(&db, cipher)?;
    driver
        .heads
        .publication(cipher, &driver.catalog, driver.catalog_cipher.as_ref())
        .publish(
            &db,
            &mut catalog,
            &head,
            Some(&previous),
            Some(stage.batch.outbox()[0].entry()),
        )?;
    db.commit().map_err(sql_error)?;
    driver.catalog.observe(&catalog)?;
    Ok(stage.candidate)
}
struct SnapshotCapture {
    archive: JournalSnapshot,
    source: SnapshotProjectionSource,
    allocation: AllocationLease,
}
impl SnapshotCapture {
    fn load(
        db: &Connection,
        cipher: &dyn JournalCipher,
        head: &head::Head,
        active: Option<&codec::Candidate>,
        work: &dyn PublicationWorkBudget,
        limits: SnapshotLimits,
    ) -> Result<Self, JournalError> {
        let covered = head.position();
        let allocation = capture_allowance(work, limits)?;
        let records = native_records(db, cipher, head, limits)?;
        // A generic cached SDK state is not an original ModelAction checkpoint.
        // With no actual action, preserve all native reservations/facts and encode
        // the explicit absent anchor/checkpoint rather than inventing its context.
        let checkpoint = if records.iter().any(|stored| {
            matches!(
                stored.record().kind(),
                JournalRecordKind::ModelAction { .. }
            )
        }) {
            native_checkpoint(db, cipher, &head)?
        } else {
            None
        };
        let archive =
            NativeRunArchive::new(head.lease().stream().clone(), covered, records, checkpoint)?;
        let snapshot =
            JournalSnapshot::new(head.lease().stream().clone(), covered, archive.encode()?)?;

        let source = public_source(db, cipher, head, active, work, limits)?;
        Ok(Self {
            archive: snapshot,
            source,
            allocation,
        })
    }
}
struct StagedSnapshot {
    candidate: SnapshotCandidate,
    batch: JournalBatch,
    archive: JournalSnapshot,
    _allocation: AllocationLease,
}
impl StagedSnapshot {
    fn encode(
        captured: SnapshotCapture,
        lease: &CleanupLease,
        encoder: &dyn CoordinatorSnapshotEncoder,
        work: &dyn PublicationWorkBudget,
        limits: SnapshotLimits,
    ) -> Result<Self, JournalError> {
        let covered = captured.archive.at();
        let next = covered.next(1, 1, false)?;
        let publication = encoder.encode(lease.writer(), next, &captured.source, limits, work)?;
        if publication.stream() != lease.writer().stream()
            || publication.key_epoch() != captured.source.material()
            || publication.sealed().len() > limits.output()
            || publication.anchor().position().writer() != lease.writer().epoch()
            || publication.anchor().run().is_some()
            || publication.anchor().ciphertext().as_bytes().as_slice()
                != Sha256::digest(publication.sealed().as_bytes()).as_slice()
        {
            return Err(JournalError::InvalidBatch);
        }
        let candidate = SnapshotCandidate::new(
            &publication,
            covered,
            Fingerprint::from_bytes(Sha256::digest(captured.archive.payload().as_bytes()).into()),
            captured.archive.payload().len(),
        )?;
        let marker = JournalRecord::new(
            RecordContext::Chat,
            JournalRecordKind::PublicSnapshot,
            OpaqueBytes::new(Vec::<u8>::new())?,
        )?;
        let batch = JournalBatch::new(
            lease.writer().clone(),
            covered,
            None,
            vec![marker],
            None,
            vec![publication],
        )?;

        Ok(Self {
            candidate,
            batch,
            archive: captured.archive,
            _allocation: captured.allocation,
        })
    }
    fn persist(
        &self,
        db: &Connection,
        cipher: &dyn JournalCipher,
        head: &mut head::Head,
        chat_bytes: u64,
    ) -> Result<(), JournalError> {
        let charge = head
            .budget()
            .appended(&self.batch)?
            .maintenance(self.archive.payload().len(), self.candidate.length())?;
        if charge.total() > chat_bytes {
            return Err(JournalError::Capacity);
        }
        super::super::append::records(db, cipher, &self.batch)?;
        super::super::append::outbox(db, cipher, &self.batch)?;
        storage::put(
            db,
            cipher,
            &self.candidate,
            1,
            &native::Snapshot::from(&self.archive),
        )?;
        storage::put(
            db,
            cipher,
            &self.candidate,
            2,
            &native::Outbox::from(self.batch.outbox()[0].entry()),
        )?;

        head.update(self.batch.next_position()?, head.deleted(), charge);
        Ok(())
    }
}
fn capture_allowance(
    work: &dyn PublicationWorkBudget,
    limits: SnapshotLimits,
) -> Result<AllocationLease, JournalError> {
    // Reserve explicit simultaneous ceilings before any transcript generation:
    // source owned records/metadata; prior native archive plaintext+decoded;
    // checkpoint bytes; archive output+per-record serialization; native archive
    // seal plaintext+ciphertext; public output's borrowed native envelope clone,
    // seal plaintext+ciphertext; and bounded small transfer/header metadata.
    let native_archive = MAX_ARCHIVE_BYTES;
    let archive_plain = native_archive;
    let archive_decode = native_archive;
    let checkpoint = native_archive;
    let archive_output = native_archive;
    let record_serialization = native_archive;
    let archive_seal_plain = native_archive + 64 * 1024;
    let archive_seal_cipher = native_archive + 64 * 1024;
    let public_clone = limits.output();
    let public_seal_plain = limits.output() + 64 * 1024;
    let public_seal_cipher = limits.output() + 64 * 1024;
    let source_metadata = limits
        .records()
        .checked_mul(16 * 1024)
        .ok_or(JournalError::Capacity)?;
    let bytes = [
        limits.bytes(),
        source_metadata,
        archive_plain,
        archive_decode,
        checkpoint,
        archive_output,
        native_archive,
        record_serialization,
        archive_seal_plain,
        archive_seal_cipher,
        public_clone,
        public_seal_plain,
        public_seal_cipher,
        64 * 1024,
    ]
    .into_iter()
    .try_fold(0usize, |n, b| {
        n.checked_add(b).ok_or(JournalError::Capacity)
    })?;
    work.try_reserve(PublicationAllocation::new(bytes, 1)?)
}
fn public_source(
    db: &Connection,
    cipher: &dyn JournalCipher,
    head: &head::Head,
    active: Option<&codec::Candidate>,
    work: &dyn PublicationWorkBudget,
    limits: SnapshotLimits,
) -> Result<SnapshotProjectionSource, JournalError> {
    let covered = head.position();
    let prior = active
        .map(|active| {
            let candidate = active.clone().restore()?;
            let (stored, allocation) =
                storage::get::<native::Outbox>(db, cipher, &candidate, 3, work)?;
            ResidentPublication::new(candidate.stream().clone(), stored.restore()?, allocation)
        })
        .transpose()?;
    let after = prior
        .as_ref()
        .map_or(EventSequence::default(), ResidentPublication::sequence);
    let mut events = Vec::new();
    let mut at = after;
    let mut public_bytes = 0usize;
    while at < covered.events() {
        if events.len() >= limits.records() {
            return Err(JournalError::Capacity);
        }
        let envelope = storage::outbox_length(db, head.lease().stream(), at.advance(1)?)?;
        let allocation = storage::reserve_decode(work, envelope)?;
        let entry = super::super::replay::outbox_from(
            db,
            cipher,
            head.lease().stream(),
            head,
            at,
            JournalReadLimits::new(1, limits.bytes())?,
        )?
        .pop()
        .ok_or(JournalError::Corrupt)?;
        public_bytes = public_bytes
            .checked_add(entry.sealed().len())
            .ok_or(JournalError::Capacity)?;
        if public_bytes > limits.bytes() {
            return Err(JournalError::Capacity);
        }
        at = entry.sequence();
        events.push(ResidentPublication::new(
            head.lease().stream().clone(),
            entry,
            allocation,
        )?);
    }
    SnapshotProjectionSource::new(
        head.lease().stream().clone(),
        cipher.chat_key_epoch(head.lease().stream())?,
        covered,
        prior,
        events,
        limits,
    )
}
fn native_checkpoint(
    db: &Connection,
    cipher: &dyn JournalCipher,
    head: &head::Head,
) -> Result<Option<nocterm_cloud_model::engine::EngineCheckpoint>, JournalError> {
    super::super::replay::object(
        db,
        head.lease().stream(),
        JournalStorageClass::Checkpoint,
        head.position(),
    )?
    .map(|(context, body)| native::open::<native::Checkpoint>(cipher, &context, &body)?.restore())
    .transpose()
}
fn native_records(
    db: &Connection,
    cipher: &dyn JournalCipher,
    head: &head::Head,
    limits: SnapshotLimits,
) -> Result<Vec<StoredRecord>, JournalError> {
    let mut records = Vec::new();
    let mut after = JournalSequence::default();
    if head.retained().get() > 0 {
        let (context, body) = super::super::replay::object(
            db,
            head.lease().stream(),
            JournalStorageClass::Snapshot,
            head.position(),
        )?
        .ok_or(JournalError::Corrupt)?;
        let snapshot = native::open::<native::Snapshot>(cipher, &context, &body)?.restore()?;
        if snapshot.at().records() != head.retained() {
            return Err(JournalError::Corrupt);
        }
        after = snapshot.at().records();
        records = NativeRunArchive::decode(&snapshot)?.into_records();
    }
    let mut bytes = records.iter().try_fold(0usize, |n, r| {
        n.checked_add(r.record().budget_bytes())
            .ok_or(JournalError::Capacity)
    })?;
    while after < head.position().records() {
        let page = super::super::replay::read_from(
            db,
            cipher,
            head.lease().stream(),
            head,
            after,
            JournalReadLimits::new(64, limits.bytes())?,
        )?;
        after = page.next();
        for record in page.records() {
            bytes = bytes
                .checked_add(record.record().budget_bytes())
                .ok_or(JournalError::Capacity)?;
            if bytes > limits.bytes() || records.len() >= limits.records() {
                return Err(JournalError::Capacity);
            }
            records.push(record.clone());
        }
    }
    Ok(records)
}
