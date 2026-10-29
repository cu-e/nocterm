//! Serial per-chat publication, with no model or target wait inside its gate.
use super::io::{JournalGuard, JournalIo};
use futures::lock::Mutex;
use nocterm_cloud_model::{
    authority::StreamBinding,
    coordinator::{CoordinatorError, CoordinatorEvent, CoordinatorEventEncoder},
    engine::EngineCheckpoint,
    journal::*,
};
use parking_lot::Mutex as SyncMutex;
use std::sync::Arc;

pub(super) struct ChatLog {
    io: JournalIo,
    lease: WriterLease,
    position: SyncMutex<JournalPosition>,
    gate: Mutex<()>,
    encoder: Arc<dyn CoordinatorEventEncoder>,
    work: crate::CoordinatorPublicationBudgets,
}
impl ChatLog {
    pub(super) async fn claim(
        io: JournalIo,
        stream: StreamBinding,
        encoder: Arc<dyn CoordinatorEventEncoder>,
        work: crate::CoordinatorPublicationBudgets,
    ) -> Result<Self, CoordinatorError> {
        let lookup = stream.clone();
        let head = io.run(move |journal| journal.head(lookup)).await?;
        if head.as_ref().is_some_and(JournalHead::deleted) {
            return Err(CoordinatorError::Stopped);
        }
        let previous = head.as_ref().map(|head| head.lease().epoch());
        let next = match previous {
            Some(epoch) => epoch.next()?,
            None => nocterm_cloud_model::identity::WriterEpoch::new(1)?,
        };
        let claim = WriterClaim::new(stream, previous, next)?;
        let lease = io.run(move |journal| journal.claim_writer(claim)).await?;
        io.remember_writer(lease.clone())?;
        Ok(Self {
            io,
            lease,
            position: SyncMutex::new(
                head.map_or_else(JournalPosition::default, |head| head.position()),
            ),
            gate: Mutex::new(()),
            encoder,
            work,
        })
    }
    pub(super) fn lease(&self) -> &WriterLease {
        &self.lease
    }
    pub(super) fn position(&self) -> JournalPosition {
        *self.position.lock()
    }
    pub(super) fn require_current(&self) -> Result<(), CoordinatorError> {
        self.io
            .journal()
            .require_current(&self.lease)
            .map_err(CoordinatorError::from)
    }
    pub(super) async fn synchronize(&self) -> Result<(), CoordinatorError> {
        let _chat = self.gate.lock().await;
        let disk = self.io.acquire().await?;
        self.refresh(&disk).await
    }
    pub(super) async fn receipt(
        &self,
        command: CommandAcceptance,
    ) -> Result<Option<CommandReceipt>, CoordinatorError> {
        let stream = self.lease.stream().clone();
        let id = command.id();
        let existing = self
            .io
            .run(move |journal| journal.command_receipt(stream, id))
            .await?;
        if let Some(receipt) = &existing {
            classify_command(
                self.lease.stream(),
                Some(receipt),
                &command,
                self.position().control(),
            )?;
        }
        Ok(existing)
    }
    /// Every event corresponds to exactly one record, encoded at its actual
    /// next journal/event position. The public encoder remains the sole wire owner.
    pub(super) async fn append(
        &self,
        command: Option<CommandAcceptance>,
        records: Vec<(JournalRecord, Option<CoordinatorEvent>)>,
        checkpoint: Option<EngineCheckpoint>,
    ) -> Result<CommitOutcome, CoordinatorError> {
        self.append_with(command, records, checkpoint, None, None)
            .await
    }
    pub(super) async fn append_resident(
        &self,
        command: CommandAcceptance,
        records: Vec<(JournalRecord, Option<CoordinatorEvent>)>,
        allocation: nocterm_core::allocation::AllocationLease,
    ) -> Result<CommitOutcome, CoordinatorError> {
        self.append_with(Some(command), records, None, Some(allocation), None)
            .await
    }
    pub(super) async fn append_after_command(
        &self,
        original: &CommandReceipt,
        records: Vec<(JournalRecord, Option<CoordinatorEvent>)>,
    ) -> Result<CommitOutcome, CoordinatorError> {
        let kind = original.acceptance().kind();
        if original.lease().stream() != self.lease.stream()
            || !crate::CoordinatorPublicationBudgets::is_urgent(kind)
        {
            return Err(JournalError::InvalidBatch.into());
        }
        self.append_with(None, records, None, None, Some(kind))
            .await
    }
    async fn append_with(
        &self,
        command: Option<CommandAcceptance>,
        records: Vec<(JournalRecord, Option<CoordinatorEvent>)>,
        checkpoint: Option<EngineCheckpoint>,
        allocation: Option<nocterm_core::allocation::AllocationLease>,
        after: Option<CommandKind>,
    ) -> Result<CommitOutcome, CoordinatorError> {
        let _guard = self.gate.lock().await;
        let disk = if let Some(allocation) = allocation {
            self.io.acquire_resident(allocation).await?
        } else {
            self.io.acquire().await?
        };
        self.refresh(&disk).await?;
        let budget = self
            .work
            .for_command(command.as_ref().map(CommandAcceptance::kind).or(after))
            .clone();
        self.commit_locked(&disk, command, records, checkpoint, budget)
            .await
    }
    pub(super) async fn model_action(
        &self,
        record: JournalRecord,
        event: CoordinatorEvent,
        engine: &mut dyn nocterm_cloud_model::engine::AgentEngine,
    ) -> Result<CommitOutcome, CoordinatorError> {
        let _guard = self.gate.lock().await;
        let disk = self.io.acquire().await?;
        self.refresh(&disk).await?;
        let next = self.position().next(1, 1, record.changes_control())?;
        let checkpoint = engine.checkpoint(next.revision())?;
        self.commit_locked(
            &disk,
            None,
            vec![(record, Some(event))],
            Some(checkpoint),
            self.work.ordinary().clone(),
        )
        .await
    }
    async fn refresh(&self, disk: &JournalGuard) -> Result<(), CoordinatorError> {
        let stream = self.lease.stream().clone();
        let head = self
            .io
            .locked(disk, move |journal| journal.head(stream))
            .await?
            .ok_or(JournalError::NotFound)?;
        if head.lease() != &self.lease || head.deleted() {
            return Err(JournalError::Fenced.into());
        }
        let mut position = self.position.lock();
        if head.position().revision() < position.revision()
            || head.position().records() < position.records()
            || head.position().events() < position.events()
            || head.position().control() < position.control()
        {
            return Err(JournalError::Corrupt.into());
        }
        *position = head.position();
        Ok(())
    }
    async fn commit_locked(
        &self,
        disk: &JournalGuard,
        command: Option<CommandAcceptance>,
        records: Vec<(JournalRecord, Option<CoordinatorEvent>)>,
        checkpoint: Option<EngineCheckpoint>,
        budget: Arc<dyn PublicationWorkBudget>,
    ) -> Result<CommitOutcome, CoordinatorError> {
        let expected = self.position();
        if let Some(command) = &command {
            let stream = self.lease.stream().clone();
            let id = command.id();
            if let Some(receipt) = self
                .io
                .locked(disk, move |journal| journal.command_receipt(stream, id))
                .await?
            {
                classify_command(
                    self.lease.stream(),
                    Some(&receipt),
                    command,
                    expected.control(),
                )?;
                return Ok(CommitOutcome::Duplicate(Box::new(receipt)));
            }
        }
        let count = records.len();
        let mut events = 0usize;
        let mut outbox = Vec::new();
        let mut raw = Vec::with_capacity(count);
        // The batch computes whether control changes; encode only after that
        // checked semantic position is known, without provisional signed bytes.
        for (record, _) in &records {
            raw.push(record.clone());
        }
        let control = records.iter().any(|(record, _)| record.changes_control());
        let event_count = records.iter().filter(|(_, event)| event.is_some()).count();
        let next = expected.next(count, event_count, control)?;
        for (index, (_, event)) in records.iter().enumerate() {
            if let Some(event) = event {
                events += 1;
                let at = JournalPosition::new(
                    next.revision(),
                    next.control(),
                    expected.records().advance(index + 1)?,
                    expected.events().advance(events)?,
                );
                outbox.push(
                    self.encoder
                        .encode(&self.lease, at, event, budget.as_ref())?,
                );
            }
        }
        let batch = JournalBatch::new(
            self.lease.clone(),
            expected,
            command,
            raw,
            checkpoint,
            outbox,
        )?;
        if batch.next_position()? != next {
            return Err(JournalError::InvalidBatch.into());
        }
        let outcome = self
            .io
            .locked(disk, move |journal| journal.commit(batch))
            .await?;
        if let CommitOutcome::Committed(receipt) = &outcome {
            *self.position.lock() = receipt.position();
        }
        Ok(outcome)
    }
}
