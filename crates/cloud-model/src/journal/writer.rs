use super::{CommitOutcome, JournalBatch, JournalError, JournalFuture};
use crate::{authority::StreamBinding, identity::WriterEpoch};

/// A native journal's returned fencing observation; this value does not acquire a lock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriterLease {
    stream: StreamBinding,
    epoch: WriterEpoch,
}
impl WriterLease {
    pub fn new(stream: StreamBinding, epoch: WriterEpoch) -> Self {
        Self { stream, epoch }
    }
    pub fn stream(&self) -> &StreamBinding {
        &self.stream
    }
    pub fn epoch(&self) -> WriterEpoch {
        self.epoch
    }
}

#[derive(Clone, Debug)]
pub struct WriterClaim {
    stream: StreamBinding,
    expected: Option<WriterEpoch>,
    next: WriterEpoch,
}
impl WriterClaim {
    pub fn new(
        stream: StreamBinding,
        expected: Option<WriterEpoch>,
        next: WriterEpoch,
    ) -> Result<Self, JournalError> {
        match expected {
            Some(previous) if next != previous.next()? => return Err(JournalError::Fenced),
            None if next.get() != 1 => return Err(JournalError::Fenced),
            _ => {}
        }
        Ok(Self {
            stream,
            expected,
            next,
        })
    }
    pub fn stream(&self) -> &StreamBinding {
        &self.stream
    }
    pub fn expected(&self) -> Option<WriterEpoch> {
        self.expected
    }
    pub fn next(&self) -> WriterEpoch {
        self.next
    }
}

pub trait RunJournalWriter: Send + Sync {
    /// Durable CAS of the same installed runner's writer epoch. No automatic
    /// takeover of a live SSH target by a different runner is authorized. ChatIdentity
    /// is the unique storage key; runner/key epochs are checked authority, never
    /// an alternate partition permitting the same account/chat to fork.
    fn claim_writer(&self, claim: WriterClaim) -> JournalFuture<WriterLease>;
    /// One durable transaction covers immutable command admission, typed effect
    /// records, derived checkpoint and exact ciphertext outbox bytes. Same command
    /// id+semantic digest returns Duplicate(original receipt); substitutions conflict.
    /// No provider or target I/O runs while its database transaction is held.
    fn commit(&self, batch: JournalBatch) -> JournalFuture<CommitOutcome>;
}
