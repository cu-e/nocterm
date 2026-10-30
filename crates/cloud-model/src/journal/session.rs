//! Negative session facts require an exact prior authenticated establishment.
use super::{CleanupLease, JournalError, JournalFuture, JournalPosition, ResidentPublication};
use crate::{
    coordinator::{SessionObservation, SessionOutcome},
    identity::{Fingerprint, JournalSequence, StreamRevision, WriterEpoch},
    payload::OpaqueBytes,
};

/// Actual native establishment coordinates captured after commit. Construction
/// is descriptive; the adapter verifies the original authenticated record again.
#[derive(Clone)]
pub struct SessionEstablishedAnchor {
    writer: WriterEpoch,
    sequence: JournalSequence,
    revision: StreamRevision,
    observation: SessionObservation,
    payload_digest: Fingerprint,
}
impl SessionEstablishedAnchor {
    pub fn new(
        writer: WriterEpoch,
        sequence: JournalSequence,
        revision: StreamRevision,
        observation: SessionObservation,
        payload_digest: Fingerprint,
    ) -> Result<Self, JournalError> {
        if sequence.get() == 0
            || revision.get() == 0
            || !matches!(observation.outcome(), SessionOutcome::Established(_))
        {
            return Err(JournalError::InvalidBatch);
        }
        Ok(Self {
            writer,
            sequence,
            revision,
            observation,
            payload_digest,
        })
    }
    pub fn writer(&self) -> WriterEpoch {
        self.writer
    }
    pub fn sequence(&self) -> JournalSequence {
        self.sequence
    }
    pub fn revision(&self) -> StreamRevision {
        self.revision
    }
    pub fn observation(&self) -> &SessionObservation {
        &self.observation
    }
    pub fn payload_digest(&self) -> Fingerprint {
        self.payload_digest
    }
}
#[derive(Clone)]
pub struct SessionLossCleanup {
    lease: CleanupLease,
    expected: JournalPosition,
    original: SessionEstablishedAnchor,
    observation: SessionObservation,
    payload: OpaqueBytes,
    outbox: Vec<ResidentPublication>,
}
impl SessionLossCleanup {
    pub fn new(
        lease: CleanupLease,
        expected: JournalPosition,
        original: SessionEstablishedAnchor,
        observation: SessionObservation,
        payload: OpaqueBytes,
        outbox: Vec<ResidentPublication>,
    ) -> Result<Self, JournalError> {
        let SessionOutcome::Established(established) = original.observation().outcome() else {
            return Err(JournalError::InvalidBatch);
        };
        if observation.binding() != original.observation().binding()
            || observation.binding().run().stream() != *lease.writer().stream()
            || !matches!(observation.outcome(), SessionOutcome::Lost { session, .. } if session == established)
            || original.writer() > lease.writer().epoch()
            || original.sequence() > expected.records()
            || original.revision() > expected.revision()
            || payload.len() > MAX_SESSION_FACT_BYTES
            || outbox.len() != 1
            || outbox.first().is_none_or(|entry| {
                entry.anchor().kind() != super::HeadAnchorKind::Event
                    || entry.anchor().run() != Some(observation.binding().run())
                    || entry.anchor().position().writer() != lease.writer().epoch()
            })
        {
            return Err(JournalError::InvalidBatch);
        }
        Ok(Self {
            lease,
            expected,
            original,
            observation,
            payload,
            outbox,
        })
    }
    pub fn lease(&self) -> &CleanupLease {
        &self.lease
    }
    pub fn expected(&self) -> JournalPosition {
        self.expected
    }
    pub fn original(&self) -> &SessionEstablishedAnchor {
        &self.original
    }
    pub fn observation(&self) -> &SessionObservation {
        &self.observation
    }
    pub fn payload(&self) -> &OpaqueBytes {
        &self.payload
    }
    pub fn outbox(&self) -> &[ResidentPublication] {
        &self.outbox
    }
}
/// A description has only bounded identities and no transcript/tool output.
/// Native readers preflight the authenticated record before decoding this body.
pub const MAX_SESSION_FACT_BYTES: usize = 64 * 1024;

/// Same native storage and transaction owner as ordinary cleanup. This distinct
/// port cannot admit commands, grants, new sessions, execution or live receipts.
pub trait RunSessionFactCleanup: Send + Sync {
    fn binding(&self) -> &super::JournalCatalogBinding;
    /// Logical native decode/DTO peak, excluding SQLite/runtime RSS. Reserve
    /// before entering the transaction; no idle reservation is required.
    fn work_bytes(&self) -> usize;
    fn finish_session_loss(
        &self,
        batch: SessionLossCleanup,
        allocation: nocterm_core::allocation::AllocationLease,
    ) -> JournalFuture<()>;
}
