//! Narrow terminal facts and account declarations for an already admitted effect.
use super::{
    CommandAcceptance, CommandKind, CommitOutcome, JournalError, JournalFuture, JournalPosition,
    ResidentPublication, WriterClaim, WriterLease,
};
use crate::{
    identity::Fingerprint,
    payload::OpaqueBytes,
    run::ToolIntent,
    tool::{BackgroundCommand, BackgroundCommandOutcome},
};

/// Descriptive cleanup view. It cannot admit a prompt, grant, intent or dispatch.
/// The adapter checks the current actual writer again inside its transaction.
#[derive(Clone, Debug)]
pub struct CleanupLease {
    writer: WriterLease,
}
impl CleanupLease {
    pub fn for_writer(writer: WriterLease) -> Self {
        Self { writer }
    }
    pub fn writer(&self) -> &WriterLease {
        &self.writer
    }
}

/// The trusted coordinator maps an authenticated ResolveApproval-Reconcile
/// body, preserving its complete original execution coordinates and digest(M).
/// Construction itself is neither account authentication nor remote observation.
#[derive(Clone, Debug)]
pub struct CleanupDeclaration {
    command: CommandAcceptance,
    original: ToolIntent,
    declared: Fingerprint,
    admission: OpaqueBytes,
    fact: OpaqueBytes,
}
impl CleanupDeclaration {
    pub fn new(
        command: CommandAcceptance,
        original: ToolIntent,
        declared: Fingerprint,
        admission: OpaqueBytes,
        fact: OpaqueBytes,
    ) -> Result<Self, JournalError> {
        if command.kind() != CommandKind::ResolveApproval {
            return Err(JournalError::InvalidBatch);
        }
        Ok(Self {
            command,
            original,
            declared,
            admission,
            fact,
        })
    }
    pub fn command(&self) -> &CommandAcceptance {
        &self.command
    }
    pub fn original(&self) -> &ToolIntent {
        &self.original
    }
    pub fn declared_result(&self) -> Fingerprint {
        self.declared
    }
    pub fn admission_payload(&self) -> &OpaqueBytes {
        &self.admission
    }
    pub fn fact_payload(&self) -> &OpaqueBytes {
        &self.fact
    }
}
#[derive(Clone, Debug)]
pub struct CleanupReconciliation {
    lease: CleanupLease,
    expected: JournalPosition,
    declaration: CleanupDeclaration,
    outbox: Vec<ResidentPublication>,
}
impl CleanupReconciliation {
    pub fn new(
        lease: CleanupLease,
        expected: JournalPosition,
        declaration: CleanupDeclaration,
        outbox: Vec<ResidentPublication>,
    ) -> Result<Self, JournalError> {
        if &declaration.original().authority().run().stream() != lease.writer().stream()
            || outbox.len() != 2
        {
            return Err(JournalError::InvalidBatch);
        }
        Ok(Self {
            lease,
            expected,
            declaration,
            outbox,
        })
    }
    pub fn lease(&self) -> &CleanupLease {
        &self.lease
    }
    pub fn expected(&self) -> JournalPosition {
        self.expected
    }
    pub fn declaration(&self) -> &CleanupDeclaration {
        &self.declaration
    }
    pub fn outbox(&self) -> &[ResidentPublication] {
        &self.outbox
    }
}

pub trait RunJournalCleanup: Send + Sync {
    /// Only an existing deleted head may receive a new cleanup writer. Ordinary
    /// claim/append still reject that tombstone; it is never resurrected.
    fn claim_cleanup(&self, claim: WriterClaim) -> JournalFuture<CleanupLease>;
    /// Already observed terminal facts require the exact original durable
    /// retained dispatch/job and CURRENT cleanup writer. Unknown retains fences.
    /// This method never starts/polls a remote process or creates Retained.
    fn finish_background(
        &self,
        lease: CleanupLease,
        command: BackgroundCommand,
        outcome: BackgroundCommandOutcome,
    ) -> JournalFuture<()>;
    /// Original immutable receipt lookup precedes current control/epoch checks.
    /// A user declaration remains Reconciled, never ObservedExit or an invoice.
    fn reconcile(&self, batch: CleanupReconciliation) -> JournalFuture<CommitOutcome>;
}
