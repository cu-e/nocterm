use super::{
    CommandAcceptance, JournalError, JournalPosition, JournalRecord, JournalRecordKind,
    MAX_JOURNAL_RECORDS, MAX_TRANSACTION_BYTES, WriterLease,
};
use crate::{
    authority::{RunBinding, StreamBinding},
    engine::EngineCheckpoint,
    identity::{ChatControlRevision, EventId, EventSequence, JournalSequence, Revision},
    payload::OpaqueBytes,
    run::CommitStamp,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct OutboxEntry {
    anchor: super::HeadAnchor,
    key_epoch: crate::identity::ChatKeyEpoch,
    delivery_id: super::OutboxDeliveryId,
    sealed: OpaqueBytes,
}
impl OutboxEntry {
    /// The trusted public producer supplies the original signed context. The
    /// native adapter independently checks its actual ciphertext fingerprint,
    /// writer/position, original run and installed material before persistence.
    pub fn new(
        anchor: super::HeadAnchor,
        key_epoch: crate::identity::ChatKeyEpoch,
        delivery_id: super::OutboxDeliveryId,
        sealed: OpaqueBytes,
    ) -> Self {
        Self {
            anchor,
            key_epoch,
            delivery_id,
            sealed,
        }
    }
    pub fn delivery_id(&self) -> super::OutboxDeliveryId {
        self.delivery_id
    }
    pub fn anchor(&self) -> &super::HeadAnchor {
        &self.anchor
    }
    pub fn key_epoch(&self) -> crate::identity::ChatKeyEpoch {
        self.key_epoch
    }
    pub fn id(&self) -> EventId {
        self.anchor.id()
    }
    pub fn sequence(&self) -> EventSequence {
        self.anchor.position().event()
    }
    pub fn record(&self) -> JournalSequence {
        self.anchor.position().journal()
    }
    pub fn sealed(&self) -> &OpaqueBytes {
        &self.sealed
    }
}

#[derive(Clone, Debug)]
pub struct JournalBatch {
    lease: WriterLease,
    expected: JournalPosition,
    command: Option<CommandAcceptance>,
    records: Vec<JournalRecord>,
    checkpoint: Option<EngineCheckpoint>,
    outbox: Vec<super::ResidentPublication>,
}
impl JournalBatch {
    pub fn new(
        lease: WriterLease,
        expected: JournalPosition,
        command: Option<CommandAcceptance>,
        records: Vec<JournalRecord>,
        checkpoint: Option<EngineCheckpoint>,
        outbox: Vec<super::ResidentPublication>,
    ) -> Result<Self, JournalError> {
        let batch = Self {
            lease,
            expected,
            command,
            records,
            checkpoint,
            outbox,
        };
        batch.validate()?;
        Ok(batch)
    }
    pub fn lease(&self) -> &WriterLease {
        &self.lease
    }
    pub fn expected(&self) -> JournalPosition {
        self.expected
    }
    pub fn command(&self) -> Option<&CommandAcceptance> {
        self.command.as_ref()
    }
    pub fn records(&self) -> &[JournalRecord] {
        &self.records
    }
    pub fn checkpoint(&self) -> Option<&EngineCheckpoint> {
        self.checkpoint.as_ref()
    }
    pub fn outbox(&self) -> &[super::ResidentPublication] {
        &self.outbox
    }
    pub fn next_position(&self) -> Result<JournalPosition, JournalError> {
        self.expected.next(
            self.records.len(),
            self.outbox.len(),
            self.records.iter().any(JournalRecord::changes_control),
        )
    }
    pub fn budget_bytes(&self) -> usize {
        let records = self
            .records
            .iter()
            .fold(0usize, |n, record| n.saturating_add(record.budget_bytes()));
        let outbox = self
            .outbox
            .iter()
            .fold(0usize, |n, entry| n.saturating_add(entry.sealed().len()));
        records
            .saturating_add(outbox)
            .saturating_add(self.checkpoint.as_ref().map_or(0, |c| c.bytes().len()))
    }
    fn validate(&self) -> Result<(), JournalError> {
        if self.records.is_empty()
            || self.records.len() > MAX_JOURNAL_RECORDS
            || self.outbox.len() > MAX_JOURNAL_RECORDS
            || self.budget_bytes() > MAX_TRANSACTION_BYTES
        {
            return Err(JournalError::Capacity);
        }
        self.validate_records()?;
        self.validate_command()?;
        self.validate_checkpoint()?;
        self.validate_outbox()
    }
    fn validate_records(&self) -> Result<(), JournalError> {
        let mut revisions = BTreeMap::<&RunBinding, Revision>::new();
        for record in &self.records {
            if let Some(run) = record.context().run()
                && &run.stream() != self.lease.stream()
            {
                return Err(JournalError::InvalidBatch);
            }
            if let Some(stamp) = record.context().stamp() {
                if stamp.writer() != self.lease.epoch() {
                    return Err(JournalError::Fenced);
                }
                if let Some(previous) = revisions.insert(stamp.run(), stamp.revision())
                    && previous.next()? != stamp.revision()
                {
                    return Err(JournalError::InvalidBatch);
                }
            }
        }
        Ok(())
    }
    fn validate_command(&self) -> Result<(), JournalError> {
        let accepted = self
            .records
            .iter()
            .filter_map(|record| match record.kind() {
                JournalRecordKind::CommandAccepted { id, kind } => Some((*id, *kind)),
                _ => None,
            })
            .collect::<Vec<_>>();
        match (&self.command, accepted.as_slice()) {
            (None, []) => Ok(()),
            (Some(command), [(id, kind)]) if command.id() == *id && command.kind() == *kind => {
                Ok(())
            }
            _ => Err(JournalError::InvalidBatch),
        }
    }
    fn validate_checkpoint(&self) -> Result<(), JournalError> {
        if let Some(checkpoint) = &self.checkpoint
            && (checkpoint.journal_revision() != self.next_position()?.revision()
                || &checkpoint.run().stream() != self.lease.stream()
                || !self
                    .records
                    .iter()
                    .any(|record| record.context().run() == Some(checkpoint.run())))
        {
            return Err(JournalError::InvalidBatch);
        }
        Ok(())
    }
    fn validate_outbox(&self) -> Result<(), JournalError> {
        let position = self.next_position()?;
        let mut ids = BTreeSet::new();
        let mut published = BTreeSet::new();
        for (index, entry) in self.outbox.iter().enumerate() {
            if entry.stream() != self.lease.stream()
                || !ids.insert(entry.id())
                || entry.sequence() != self.expected.events().advance(index + 1)?
                || entry.record() <= self.expected.records()
                || entry.record() > position.records()
                || entry.sealed().is_empty()
            {
                return Err(JournalError::InvalidBatch);
            }
            let offset = usize::try_from(entry.record().get() - self.expected.records().get() - 1)
                .map_err(|_| JournalError::InvalidBatch)?;
            if (entry.anchor().kind() == super::HeadAnchorKind::Snapshot)
                != matches!(
                    self.records[offset].kind(),
                    JournalRecordKind::PublicSnapshot
                )
            {
                return Err(JournalError::InvalidBatch);
            }
            published.insert(entry.record());
        }
        for (index, record) in self.records.iter().enumerate() {
            if record.requires_outbox()
                && !published.contains(&self.expected.records().advance(index + 1)?)
            {
                return Err(JournalError::InvalidBatch);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct CommandReceipt {
    lease: WriterLease,
    position: JournalPosition,
    acceptance: CommandAcceptance,
}
impl CommandReceipt {
    /// Restore an authenticated native receipt. This description is not an
    /// acknowledgement of a new commit and cannot authorize a tool effect.
    pub fn from_durable(
        lease: WriterLease,
        position: JournalPosition,
        acceptance: CommandAcceptance,
    ) -> Result<Self, JournalError> {
        if position.revision().get() == 0
            || position.records().get() == 0
            || acceptance.expected_control().next()? != position.control()
        {
            return Err(JournalError::Corrupt);
        }
        Ok(Self {
            lease,
            position,
            acceptance,
        })
    }
    pub fn lease(&self) -> &WriterLease {
        &self.lease
    }
    pub fn position(&self) -> JournalPosition {
        self.position
    }
    pub fn acceptance(&self) -> &CommandAcceptance {
        &self.acceptance
    }
}

#[derive(Clone, Debug)]
pub struct JournalCommitReceipt {
    lease: WriterLease,
    position: JournalPosition,
    command: Option<CommandReceipt>,
    run_commits: Vec<CommitStamp>,
}
impl JournalCommitReceipt {
    /// Called by an adapter only after its atomic durable commit, not before fsync.
    pub fn committed(batch: &JournalBatch) -> Result<Self, JournalError> {
        let position = batch.next_position()?;
        Ok(Self {
            lease: batch.lease.clone(),
            position,
            command: batch.command.clone().map(|acceptance| CommandReceipt {
                lease: batch.lease.clone(),
                position,
                acceptance,
            }),
            run_commits: batch
                .records
                .iter()
                .filter_map(|record| record.context().stamp().cloned())
                .collect(),
        })
    }
    pub fn lease(&self) -> &WriterLease {
        &self.lease
    }
    pub fn position(&self) -> JournalPosition {
        self.position
    }
    pub fn command(&self) -> Option<&CommandReceipt> {
        self.command.as_ref()
    }
    pub fn run_commits(&self) -> &[CommitStamp] {
        &self.run_commits
    }
}

#[derive(Clone, Debug)]
pub enum CommitOutcome {
    Committed(Box<JournalCommitReceipt>),
    Duplicate(Box<CommandReceipt>),
}
#[derive(Clone, Debug)]
pub enum CommandAdmission {
    New,
    Duplicate(Box<CommandReceipt>),
}

/// The native journal calls this inside its transaction. Duplicate lookup MUST
/// precede current control-revision checks; a lost ACK must return its old receipt.
pub fn classify_command(
    stream: &StreamBinding,
    existing: Option<&CommandReceipt>,
    command: &CommandAcceptance,
    current: ChatControlRevision,
) -> Result<CommandAdmission, JournalError> {
    if let Some(receipt) = existing {
        if receipt.lease.stream() != stream {
            return Err(JournalError::Corrupt);
        }
        if &receipt.acceptance != command {
            return Err(JournalError::Conflict);
        }
        return Ok(CommandAdmission::Duplicate(Box::new(receipt.clone())));
    }
    if command.expected_control() != current {
        return Err(JournalError::Conflict);
    }
    Ok(CommandAdmission::New)
}
