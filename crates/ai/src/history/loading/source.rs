//! An original document source retains its reusable reader/delivery capabilities.
#![expect(
    clippy::result_large_err,
    reason = "refusals return ledger-paid values; a Box would be an unpaid allocation"
)]
use super::{NativeLoadError, NativeMaterialized, NativePhase, ReaderCompletion, ReaderOutcome};
use crate::{
    model::{
        json::{deferred::DecodeLedger, lifetime::Shared},
        retention::{
            RetentionError,
            document::{
                AllocationPlan, ConstructionRole, DocumentRetention, PendingAllocation,
                RetainedRole,
            },
        },
    },
    storage::AgentStorageError,
};
use std::sync::Mutex;
mod delivery;
mod task;
pub use delivery::{ReaderDelivery, ReaderInstallError};
pub use task::{
    ManagedReaderFuture, ReaderDocumentId, ReaderMaterializer, ReaderPreparationFailure,
};
#[derive(Debug, thiserror::Error)]
pub enum ReaderSourceError {
    #[error("the original history reader is already in use")]
    Unavailable,
    #[error("original history reader ownership is poisoned")]
    Poisoned,
    #[error(transparent)]
    Retention(#[from] RetentionError),
    #[error(transparent)]
    Load(#[from] NativeLoadError),
}
struct Idle {
    reader: ConstructionRole,
    delivery: RetainedRole,
}
struct SourceBody {
    idle: Mutex<Option<Idle>>,
    scope: DocumentRetention,
    _allocation: PendingAllocation,
}
/// Supplied explicitly with the original Item/document; serialized IDs cannot
/// discover this endpoint or manufacture its capabilities.
#[derive(Clone)]
pub struct ReaderSource(Shared<SourceBody>);
impl ReaderSource {
    pub fn new(
        mut reader: ConstructionRole,
        delivery: RetainedRole,
    ) -> Result<Self, ReaderSourceError> {
        if !reader.is_reader()
            || !delivery.is_delivery()
            || delivery.snapshot().is_some()
            || !reader.scope().same_document(delivery.scope())
        {
            return Err(RetentionError::ForeignAuthority.into());
        }
        let scope = reader.scope().clone();
        let allocation = scope.reserve_phase_metadata(AllocationPlan::shared::<SourceBody>())?;
        reader.close_phase();
        Ok(Self(Shared::new(SourceBody {
            idle: Mutex::new(Some(Idle { reader, delivery })),
            scope,
            _allocation: allocation,
        })))
    }
    pub fn same_document(&self, scope: &DocumentRetention) -> bool {
        self.0.get().scope.same_document(scope)
    }
    pub fn capture(&self) -> Result<ReaderRequest, ReaderSourceError> {
        let Idle { reader, delivery } = self
            .0
            .get()
            .idle
            .lock()
            .map_err(|_| ReaderSourceError::Poisoned)?
            .take()
            .ok_or(ReaderSourceError::Unavailable)?;
        let phase = match NativePhase::begin(reader, delivery) {
            Ok(phase) => phase,
            Err(completion) => {
                let (reader, outcome) = completion.into_parts();
                let ReaderOutcome::Failed(failed) = outcome else {
                    unreachable!("begin cannot publish")
                };
                let (delivery, error) = failed.into_parts();
                self.restore(reader, delivery)?;
                return Err(error.into());
            }
        };
        Ok(ReaderRequest {
            phase: Some(phase),
            source: self.clone(),
        })
    }
    fn restore(
        &self,
        mut reader: ConstructionRole,
        mut delivery: RetainedRole,
    ) -> Result<(), ReaderSourceError> {
        // All native locals/ledger metadata have already died. No payload Drop
        // or document transition occurs while the availability mutex is held.
        delivery.clear();
        reader.close_phase();
        if !self.same_document(reader.scope()) || !self.same_document(delivery.scope()) {
            return Err(RetentionError::ForeignAuthority.into());
        }
        let mut idle = self
            .0
            .get()
            .idle
            .lock()
            .map_err(|_| ReaderSourceError::Poisoned)?;
        if idle.is_some() {
            return Err(ReaderSourceError::Unavailable);
        }
        *idle = Some(Idle { reader, delivery });
        Ok(())
    }
    fn recycle(&self, completion: ReaderCompletion) -> Result<(), ReaderSourceError> {
        let (reader, outcome) = completion.into_parts();
        let delivery = match outcome {
            ReaderOutcome::Ready(parts) => parts.release(),
            ReaderOutcome::Failed(failed) => failed.into_parts().0,
        };
        self.restore(reader, delivery)
    }
}
/// The phase remains here while the materializer borrows it. Unwinding its
/// pending callback locals therefore precedes this owner's return to the slot.
pub struct ReaderRequest {
    phase: Option<NativePhase>,
    source: ReaderSource,
}
impl ReaderRequest {
    /// Stages only after the original shared history permit and reader capture.
    pub fn prepare_native<T: ReaderMaterializer>(
        self,
        id: ReaderDocumentId,
        factory: impl FnOnce(ReaderDocumentId, &DecodeLedger) -> Result<T, AgentStorageError>,
    ) -> Result<ManagedReaderFuture, ReaderPreparationFailure> {
        task::prepare(self, id, factory)
    }
    pub fn read(
        mut self,
        materialize: impl FnOnce(&DecodeLedger) -> Result<NativeMaterialized, AgentStorageError>,
    ) -> ReaderDelivery {
        let result = self
            .phase
            .as_mut()
            .expect("unique reader phase")
            .run(materialize);
        let completion = self
            .phase
            .take()
            .expect("unique reader terminal")
            .finish(result);
        ReaderDelivery::new(completion, self.source.clone())
    }
    pub fn fail(self, error: AgentStorageError) -> ReaderDelivery {
        self.read(|_| Err(error))
    }
}
impl Drop for ReaderRequest {
    fn drop(&mut self) {
        if let Some(phase) = self.phase.take() {
            let completion = phase.fail(AgentStorageError::Interrupted);
            let _ = self.source.recycle(completion);
        }
    }
}
