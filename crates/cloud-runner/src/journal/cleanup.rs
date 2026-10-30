//! A fact-only view of the current journal writer, preserving every tombstone.
use super::{DurableRunJournal, append, effects, head, storage};
use nocterm_cloud_model::{
    journal::*,
    tool::{BackgroundCommand, BackgroundCommandOutcome},
};
use rusqlite::TransactionBehavior;

#[derive(Clone)]
pub struct DurableJournalCleanup {
    journal: DurableRunJournal,
}
impl DurableJournalCleanup {
    pub(super) fn new(journal: DurableRunJournal) -> Self {
        Self { journal }
    }
}
impl RunSessionFactCleanup for DurableJournalCleanup {
    fn binding(&self) -> &JournalCatalogBinding {
        self.journal.binding()
    }
    fn work_bytes(&self) -> usize {
        super::session_loss::work_bytes()
    }
    fn finish_session_loss(
        &self,
        batch: SessionLossCleanup,
        allocation: nocterm_core::allocation::AllocationLease,
    ) -> JournalFuture<()> {
        let journal = self.journal.clone();
        Box::pin(async move {
            if allocation.bytes() < super::session_loss::work_bytes() || allocation.payloads() == 0
            {
                return Err(JournalError::Capacity);
            }
            // Actual blocking caller owns this future, batch and guard through
            // transaction completion, including cancellation of its receiver.
            let resident = nocterm_core::allocation::Allocated::new(batch, allocation);
            journal.with_stream(resident.value().lease().writer().stream(), |driver| {
                super::session_loss::finish(driver, resident.value())
            })
        })
    }
}
impl RunJournalCleanup for DurableJournalCleanup {
    fn claim_cleanup(&self, claim: WriterClaim) -> JournalFuture<CleanupLease> {
        let journal = self.journal.clone();
        Box::pin(async move {
            journal.with_stream(claim.stream(), |driver| {
                driver.catalog.require_stream(claim.stream())?;
                let tx = driver
                    .connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(storage)?;
                let mut catalog = driver.catalog.load(&tx, driver.catalog_cipher.as_ref())?;
                driver.catalog.verify_rows(&tx, &catalog)?;
                let previous = driver.heads.load(&tx, claim.stream())?;
                let mut current = head::required(&tx, driver.cipher.as_ref(), claim.stream())?;
                current.claim_cleanup(&claim)?;
                current.save(&tx, driver.cipher.as_ref())?;
                driver
                    .heads
                    .publication(
                        driver.cipher.as_ref(),
                        &driver.catalog,
                        driver.catalog_cipher.as_ref(),
                    )
                    .publish(&tx, &mut catalog, &current, previous.as_ref(), None)?;
                tx.commit().map_err(storage)?;
                driver.catalog.observe(&catalog)?;
                Ok(CleanupLease::for_writer(current.lease().clone()))
            })
        })
    }
    fn finish_background(
        &self,
        lease: CleanupLease,
        command: BackgroundCommand,
        outcome: BackgroundCommandOutcome,
    ) -> JournalFuture<()> {
        let journal = self.journal.clone();
        Box::pin(async move {
            journal.with_stream(lease.writer().stream(), |driver| {
                effects::cleanup_background(driver, &lease, &command, outcome)
            })
        })
    }
    fn reconcile(&self, cleanup: CleanupReconciliation) -> JournalFuture<CommitOutcome> {
        let journal = self.journal.clone();
        Box::pin(async move {
            journal.with_stream(cleanup.lease().writer().stream(), |driver| {
                let stream = cleanup.lease().writer().stream();
                driver.catalog.require_stream(stream)?;
                let cipher = driver.cipher.as_ref();
                let tx = driver
                    .connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(storage)?;
                let mut catalog = driver.catalog.load(&tx, driver.catalog_cipher.as_ref())?;
                driver.catalog.verify_rows(&tx, &catalog)?;
                let previous = driver.heads.load(&tx, stream)?;
                let mut current = head::required(&tx, cipher, stream)?;
                let declaration = cleanup.declaration();
                let command = declaration.command();
                // A lost ACK after cleanup renewal/deletion still returns the exact
                // old immutable receipt before interpreting current coordinates.
                let original = append::receipt(&tx, cipher, stream, command.id())?;
                if let CommandAdmission::Duplicate(receipt) = classify_command(
                    stream,
                    original.as_ref(),
                    command,
                    current.position().control(),
                )? {
                    return Ok(CommitOutcome::Duplicate(receipt));
                }
                current.require_cleanup(cleanup.lease())?;
                if current.position() != cleanup.expected() {
                    return Err(JournalError::Conflict);
                }
                effects::require_original(&tx, cipher, stream, declaration.original())?;
                let run = declaration.original().authority().run().clone();
                let accepted = JournalRecord::new(
                    RecordContext::Run(run.clone()),
                    JournalRecordKind::CommandAccepted {
                        id: command.id(),
                        kind: CommandKind::ResolveApproval,
                    },
                    declaration.admission_payload().clone(),
                )?;
                let fact = JournalRecord::new(
                    RecordContext::Run(run),
                    JournalRecordKind::Reconciled {
                        intent: declaration.original().id(),
                        command: command.id(),
                    },
                    declaration.fact_payload().clone(),
                )?;
                let batch = JournalBatch::new(
                    cleanup.lease().writer().clone(),
                    cleanup.expected(),
                    Some(command.clone()),
                    vec![accepted, fact],
                    None,
                    cleanup.outbox().to_vec(),
                )?;
                let next = batch.next_position()?;
                let charge = current.budget().appended(&batch)?;
                if charge.total() > driver.limits.chat_bytes
                    || next
                        .records()
                        .get()
                        .saturating_sub(current.retained().get())
                        > driver.limits.chat_records
                {
                    return Err(JournalError::Capacity);
                }
                let receipt = JournalCommitReceipt::committed(&batch)?;
                append::records(&tx, cipher, &batch)?;
                append::outbox(&tx, cipher, &batch)?;
                current.set_active(effects::active_after(&tx, cipher, &current, &batch)?);
                append::save_receipt(&tx, cipher, receipt.command().ok_or(JournalError::Corrupt)?)?;
                current.update(next, current.deleted(), charge);
                current.save(&tx, cipher)?;
                driver
                    .heads
                    .publication(cipher, &driver.catalog, driver.catalog_cipher.as_ref())
                    .publish(
                        &tx,
                        &mut catalog,
                        &current,
                        previous.as_ref(),
                        batch.outbox().last().map(ResidentPublication::entry),
                    )?;
                tx.commit().map_err(storage)?;
                driver.catalog.observe(&catalog)?;
                Ok(CommitOutcome::Committed(Box::new(receipt)))
            })
        })
    }
}
