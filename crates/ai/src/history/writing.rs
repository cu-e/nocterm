//! Original document writer capabilities loaned through physical publication.
use super::physical::SnapshotOwner;
use crate::model::{
    json::{deferred::DecodeLedger, lifetime::Shared},
    retention::{
        RetentionError,
        document::{
            AllocationPlan, ConstructionRole, DocumentRetention, PendingAllocation, RetainedRole,
        },
    },
};
use std::sync::{Mutex, MutexGuard};
mod loan;
use loan::{LatestLoan, WriterLoan};

#[derive(Debug, thiserror::Error)]
pub enum WriterCapabilityError {
    #[error("writer capabilities belong to another document")]
    ForeignScope,
    #[error("incorrect document writer capability")]
    WrongRole,
    #[error("the document writer capability is already loaned")]
    Unavailable,
    #[error("document writer capability ownership is poisoned")]
    Poisoned,
    #[error(transparent)]
    Retention(#[from] RetentionError),
}
struct Available {
    latest: Option<RetainedRole>,
    active: Option<RetainedRole>,
    writer: Option<ConstructionRole>,
}
struct ProducerBody {
    available: Mutex<Available>,
    scope: DocumentRetention,
    // The Shared block is physically freed before this phase-free credit drops.
    _allocation: PendingAllocation,
}
/// One explicitly supplied endpoint per actual document incarnation. Its handles
/// share availability, never duplicate any retained/construction capability.
#[derive(Clone)]
pub struct WriterProducer(Shared<ProducerBody>);
impl WriterProducer {
    pub fn candidate<'a>(
        &'a self,
        source: &'a RetainedRole,
        intent: ManagedWriteIntent,
    ) -> Result<CandidateOffer<'a>, WriterCapabilityError> {
        if !source.is_candidate() {
            return Err(WriterCapabilityError::WrongRole);
        }
        if !self.0.scope.same_document(source.scope()) {
            return Err(WriterCapabilityError::ForeignScope);
        }
        if source.snapshot().is_none() {
            return Err(WriterCapabilityError::Unavailable);
        }
        Ok(CandidateOffer {
            source,
            producer: self,
            intent,
        })
    }
    pub fn new(
        latest: RetainedRole,
        active: RetainedRole,
        mut writer: ConstructionRole,
    ) -> Result<Self, WriterCapabilityError> {
        if !latest.is_writer_latest() || !active.is_writer_active() || !writer.is_writer() {
            return Err(WriterCapabilityError::WrongRole);
        }
        if !latest.scope().same_document(active.scope())
            || !latest.scope().same_document(writer.scope())
        {
            return Err(WriterCapabilityError::ForeignScope);
        }
        if latest.snapshot().is_some() || active.snapshot().is_some() {
            return Err(WriterCapabilityError::Unavailable);
        }
        let scope = latest.scope().clone();
        let allocation = scope.reserve_phase_metadata(AllocationPlan::shared::<ProducerBody>())?;
        writer.close_phase();
        Ok(Self(Shared::new(ProducerBody {
            available: Mutex::new(Available {
                latest: Some(latest),
                active: Some(active),
                writer: Some(writer),
            }),
            scope,
            _allocation: allocation,
        })))
    }
    fn available(&self) -> Result<MutexGuard<'_, Available>, WriterCapabilityError> {
        self.0
            .get()
            .available
            .lock()
            .map_err(|_| WriterCapabilityError::Poisoned)
    }
    pub fn same_document(&self, other: &Self) -> bool {
        self.0.get().scope.same_document(&other.0.get().scope)
    }
    /// The previous value loses its original Latest loan only after its checked
    /// role transition succeeds. Store installs the returned value synchronously.
    pub fn latest(
        &self,
        source: &RetainedRole,
        previous: Option<&mut ManagedLatest>,
        intent: ManagedWriteIntent,
    ) -> Result<ManagedLatest, WriterCapabilityError> {
        if !self.0.get().scope.same_document(source.scope()) {
            return Err(WriterCapabilityError::ForeignScope);
        }
        if let Some(previous) = previous.filter(|value| self.same_document(&value.producer)) {
            let loan = previous
                .loan
                .as_mut()
                .ok_or(WriterCapabilityError::Unavailable)?;
            loan.role_mut()?.clone_from_role(source)?;
            return Ok(ManagedLatest {
                loan: previous.loan.take(),
                intent,
                producer: self.clone(),
            });
        }
        let role = self
            .available()?
            .latest
            .take()
            .ok_or(WriterCapabilityError::Unavailable)?;
        let mut loan = LatestLoan::new(role, self.clone());
        loan.role_mut()?.clone_from_role(source)?;
        Ok(ManagedLatest {
            loan: Some(loan),
            intent,
            producer: self.clone(),
        })
    }
    fn capture(&self, latest: &ManagedLatest) -> Result<WriterLoan, WriterCapabilityError> {
        if !self.same_document(&latest.producer) {
            return Err(WriterCapabilityError::ForeignScope);
        }
        let (active, writer) = {
            let mut available = self.available()?;
            if available.active.is_none() || available.writer.is_none() {
                return Err(WriterCapabilityError::Unavailable);
            }
            (
                available.active.take().expect("checked original active"),
                available.writer.take().expect("checked original writer"),
            )
        };
        let mut loan = WriterLoan::new(active, writer, self.clone());
        loan.start(
            latest
                .loan
                .as_ref()
                .ok_or(WriterCapabilityError::Unavailable)?
                .role()?,
        )?;
        Ok(loan)
    }
}
/// Inline framing; draft bytes live in the registered snapshot's composer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagedWriteIntent {
    Snapshot,
    ArchiveDraft { updated: u64 },
}
/// A synchronous borrow of the original Candidate and writer endpoint. It
/// cannot escape a view borrow into deferred work or renew any capability.
pub struct CandidateOffer<'a> {
    source: &'a RetainedRole,
    producer: &'a WriterProducer,
    intent: ManagedWriteIntent,
}
impl CandidateOffer<'_> {
    pub fn id(&self) -> Result<super::physical::TextGuard<'_>, WriterCapabilityError> {
        Ok(self
            .source
            .snapshot()
            .ok_or(WriterCapabilityError::Unavailable)?
            .id())
    }
    pub fn into_latest(
        self,
        previous: Option<&mut ManagedLatest>,
    ) -> Result<ManagedLatest, WriterCapabilityError> {
        self.producer.latest(self.source, previous, self.intent)
    }
}
/// Latest payload is owned only through its genuine role, never a redundant Arc.
pub struct ManagedLatest {
    loan: Option<LatestLoan>,
    intent: ManagedWriteIntent,
    producer: WriterProducer,
}
impl ManagedLatest {
    pub fn capture(&self) -> Result<ManagedWrite, WriterCapabilityError> {
        Ok(ManagedWrite {
            intent: self.intent,
            loan: self.producer.capture(self)?,
        })
    }
    pub fn snapshot(&self) -> Result<&SnapshotOwner, WriterCapabilityError> {
        self.loan
            .as_ref()
            .ok_or(WriterCapabilityError::Unavailable)?
            .role()?
            .snapshot()
            .ok_or(WriterCapabilityError::Unavailable)
    }
}
/// Inseparable physical request: its native future owns this unique loan.
pub struct ManagedWrite {
    intent: ManagedWriteIntent,
    loan: WriterLoan,
}
impl ManagedWrite {
    pub fn id(&self) -> Result<super::physical::TextGuard<'_>, WriterCapabilityError> {
        Ok(self.loan.snapshot()?.id())
    }
    /// Adapter locals (held files, encoders and staging) die before this method
    /// retires the scoped ledger and returns the original writer capabilities.
    pub fn write(
        self,
        operation: impl FnOnce(
            &DecodeLedger,
            &SnapshotOwner,
            ManagedWriteIntent,
        ) -> Result<(), crate::storage::AgentStorageError>,
    ) -> Result<(), crate::storage::AgentStorageError> {
        let ledger = DecodeLedger::for_operation(self.loan.writer()?, super::MAX_RESIDENT_BYTES)
            .map_err(|error| {
                crate::storage::AgentStorageError::Codec(super::HistoryCodecError::from(error))
            })?;
        let result = operation(&ledger, self.loan.snapshot()?, self.intent);
        drop(ledger);
        result
    }
}
impl From<WriterCapabilityError> for crate::storage::AgentStorageError {
    fn from(error: WriterCapabilityError) -> Self {
        Self::Codec(super::HistoryCodecError::Validation(
            nocterm_core::SharedCause::new(error),
        ))
    }
}
