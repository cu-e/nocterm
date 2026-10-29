//! Durable receipt and prefix activation preserve the latest writer and tail.
use super::*;
use crate::codec as native;

pub(super) fn record_stored(
    driver: &mut Driver,
    lease: &CleanupLease,
    receipt: &OutboxStored,
) -> Result<SnapshotCandidate, JournalError> {
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
    let candidate = state
        .pending
        .take()
        .ok_or(JournalError::NotFound)?
        .restore()?
        .record_stored(receipt.clone())?;
    if head.acknowledged().advance(1)? != candidate.anchor().position().event()
        && head.acknowledged() < candidate.anchor().position().event()
    {
        return Err(JournalError::Conflict);
    }
    head.acknowledge(candidate.anchor().position().event());
    state.pending = Some((&candidate).into());
    storage::save(&db, cipher, &mut head, &state)?;
    head.save(&db, cipher)?;
    driver
        .heads
        .publication(cipher, &driver.catalog, driver.catalog_cipher.as_ref())
        .publish(&db, &mut catalog, &head, Some(&previous), None)?;
    db.commit().map_err(sql_error)?;
    driver.catalog.observe(&catalog)?;
    Ok(candidate)
}
pub(super) fn activate(
    driver: &mut Driver,
    lease: &CleanupLease,
    purged: &SnapshotPurged,
    work: &dyn PublicationWorkBudget,
) -> Result<ResidentSnapshot, JournalError> {
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
    let Some(candidate) = state.pending.take() else {
        return ActivatedPrefix::existing(
            &db,
            cipher,
            &head,
            state.active.ok_or(JournalError::NotFound)?.restore()?,
            purged,
            work,
        );
    };
    let prefix = ActivatedPrefix::load(&db, cipher, &head, candidate.restore()?, purged, work)?;
    let charge = prefix.charge(&db, cipher, &head, work)?;
    prefix.persist(&db, cipher, &mut head, charge)?;
    state.active = Some((&prefix.candidate).into());
    storage::save(&db, cipher, &mut head, &state)?;
    head.save(&db, cipher)?;
    driver
        .heads
        .publication(cipher, &driver.catalog, driver.catalog_cipher.as_ref())
        .publish_baseline(
            &db,
            &mut catalog,
            &head,
            &previous,
            prefix.candidate.anchor(),
        )?;
    db.commit().map_err(sql_error)?;
    driver.catalog.observe(&catalog)?;
    Ok(prefix.archive)
}
struct ActivatedPrefix {
    candidate: SnapshotCandidate,
    archive: ResidentSnapshot,
    public: ResidentPublication,
}
impl ActivatedPrefix {
    fn load(
        db: &Connection,
        cipher: &dyn JournalCipher,
        head: &head::Head,
        candidate: SnapshotCandidate,
        purged: &SnapshotPurged,
        work: &dyn PublicationWorkBudget,
    ) -> Result<Self, JournalError> {
        if candidate.stored() != Some(purged.stored())
            || purged.through().checked_add(1) != Some(purged.stored().cursor().get())
            || candidate.material() != cipher.chat_key_epoch(head.lease().stream())?
        {
            return Err(JournalError::Conflict);
        }
        let (archive, archive_allocation) =
            storage::get::<native::Snapshot>(db, cipher, &candidate, 1, work)?;
        let archive = archive.restore()?;
        if archive.at() != candidate.covered()
            || archive.stream() != candidate.stream()
            || archive.payload().len() != candidate.archive_length()
            || Sha256::digest(archive.payload().as_bytes()).as_slice()
                != candidate.archive().as_bytes()
        {
            return Err(JournalError::Corrupt);
        }
        let (public, public_allocation) =
            storage::get::<native::Outbox>(db, cipher, &candidate, 2, work)?;
        let public = public.restore()?;
        if public.anchor() != candidate.anchor()
            || public.delivery_id() != candidate.delivery_id()
            || public.key_epoch() != candidate.material()
            || public.sealed().len() != candidate.length()
            || Sha256::digest(public.sealed().as_bytes()).as_slice()
                != candidate.anchor().ciphertext().as_bytes()
        {
            return Err(JournalError::Corrupt);
        }
        let covered = candidate.covered();
        if head.position().records() < candidate.anchor().position().journal()
            || head.position().events() < candidate.anchor().position().event()
            || covered.records() < head.retained()
        {
            return Err(JournalError::Conflict);
        }

        let archive = Arc::new(ChargedSnapshot::new(archive, archive_allocation)?);
        let public =
            ResidentPublication::new(candidate.stream().clone(), public, public_allocation)?;
        Ok(Self {
            candidate,
            archive,
            public,
        })
    }
    fn existing(
        db: &Connection,
        cipher: &dyn JournalCipher,
        head: &head::Head,
        active: SnapshotCandidate,
        purged: &SnapshotPurged,
        work: &dyn PublicationWorkBudget,
    ) -> Result<ResidentSnapshot, JournalError> {
        if active.stored() != Some(purged.stored()) {
            return Err(JournalError::Conflict);
        }
        let length: i64 = db
            .query_row(
                "SELECT length(body) FROM snapshots WHERE chat=?1",
                [head::key(active.stream()).as_slice()],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        let allocation = storage::reserve_decode(work, head::check_length(length)?)?;
        let (context, body) = super::super::replay::object(
            db,
            active.stream(),
            JournalStorageClass::Snapshot,
            head.position(),
        )?
        .ok_or(JournalError::Corrupt)?;
        let archive = native::open::<native::Snapshot>(cipher, &context, &body)?.restore()?;
        if archive.at() != active.covered()
            || archive.stream() != active.stream()
            || archive.payload().len() != active.archive_length()
            || Sha256::digest(archive.payload().as_bytes()).as_slice()
                != active.archive().as_bytes()
        {
            return Err(JournalError::Corrupt);
        }
        Ok(Arc::new(ChargedSnapshot::new(archive, allocation)?))
    }

    fn charge(
        &self,
        db: &Connection,
        cipher: &dyn JournalCipher,
        head: &head::Head,
        work: &dyn PublicationWorkBudget,
    ) -> Result<crate::storage::StorageBudget, JournalError> {
        let removed_records: i64 = db
            .query_row(
                "SELECT coalesce(sum(charge),0) FROM entries WHERE chat=?1 AND sequence<=?2",
                params![
                    head::key(self.candidate.stream()).as_slice(),
                    integer(self.candidate.covered().records().get())?
                ],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        // Native envelope sizes are not public payload charges. Decode each bounded
        // covered row to release its actual original logical ciphertext charge.
        let mut after = head.event_floor();
        let mut removed_outbox = 0u64;
        while after < self.candidate.covered().events() {
            let length = storage::outbox_length(db, self.candidate.stream(), after.advance(1)?)?;
            let _allocation = storage::reserve_decode(work, length)?;
            let entry = super::super::replay::outbox_from(
                db,
                cipher,
                self.candidate.stream(),
                head,
                after,
                JournalReadLimits::new(1, MAX_TRANSACTION_BYTES)?,
            )?
            .pop()
            .ok_or(JournalError::Corrupt)?;
            removed_outbox = removed_outbox
                .checked_add(entry.sealed().len() as u64)
                .ok_or(JournalError::Corrupt)?;
            after = entry.sequence();
        }
        head.budget().activated(
            super::super::unsigned(removed_records)?,
            removed_outbox,
            self.archive.payload().len(),
            self.public.sealed().len(),
        )
    }
    fn persist(
        &self,
        db: &Connection,
        cipher: &dyn JournalCipher,
        head: &mut head::Head,
        charge: crate::storage::StorageBudget,
    ) -> Result<(), JournalError> {
        let context = head::context(
            self.candidate.stream(),
            JournalStorageClass::Snapshot,
            [0; 16],
            self.candidate.covered(),
            self.candidate.anchor().position().writer(),
        );
        let body = native::seal(
            cipher,
            &context,
            &native::Snapshot::from(self.archive.snapshot()),
        )?;
        db.execute("INSERT INTO snapshots(chat,revision,sequence,writer,body) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(chat) DO UPDATE SET revision=excluded.revision,sequence=excluded.sequence,writer=excluded.writer,body=excluded.body",params![head::key(self.candidate.stream()).as_slice(),integer(self.candidate.covered().revision().get())?,integer(self.candidate.covered().records().get())?,integer(context.writer().get())?,body]).map_err(sql_error)?;
        db.execute(
            "DELETE FROM snapshot_payloads WHERE chat=?1 AND kind=3",
            [head::key(self.candidate.stream()).as_slice()],
        )
        .map_err(sql_error)?;
        db.execute(
            "UPDATE snapshot_payloads SET kind=3 WHERE chat=?1 AND kind=2",
            [head::key(self.candidate.stream()).as_slice()],
        )
        .map_err(sql_error)?;
        db.execute(
            "DELETE FROM snapshot_payloads WHERE chat=?1 AND kind=1",
            [head::key(self.candidate.stream()).as_slice()],
        )
        .map_err(sql_error)?;
        db.execute(
            "DELETE FROM entries WHERE chat=?1 AND sequence<=?2",
            params![
                head::key(self.candidate.stream()).as_slice(),
                integer(self.candidate.covered().records().get())?
            ],
        )
        .map_err(sql_error)?;
        db.execute(
            "DELETE FROM outbox WHERE chat=?1 AND sequence<=?2",
            params![
                head::key(self.candidate.stream()).as_slice(),
                integer(self.candidate.covered().events().get())?
            ],
        )
        .map_err(sql_error)?;
        // Delivery receipts are needed only while their exact outbox body is
        // retained. The active Snapshot receipt remains in its protected phase.
        db.execute(
            "DELETE FROM outbox_deliveries WHERE chat=?1 AND sequence<=?2",
            params![
                head::key(self.candidate.stream()).as_slice(),
                integer(self.candidate.covered().events().get())?
            ],
        )
        .map_err(sql_error)?;
        head.compacted(
            self.candidate.covered().records(),
            self.candidate.covered().events(),
        );
        head.update(head.position(), head.deleted(), charge);

        Ok(())
    }
}
