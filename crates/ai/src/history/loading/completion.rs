//! Completion returns the original reader and delivery capabilities on every path.
use crate::{
    history::{
        HistoryCodecError,
        physical::{ManagedParts, PhysicalError},
    },
    model::retention::{
        RetentionError,
        document::{ConstructionRole, RetainedRole},
    },
    storage::AgentStorageError,
};

#[derive(Debug, thiserror::Error)]
pub enum NativeLoadError {
    #[error("history storage failed: {0}")]
    Storage(#[from] AgentStorageError),
    #[error("history format failed: {0}")]
    Codec(#[from] HistoryCodecError),
    #[error("history physical admission failed: {0}")]
    Physical(#[from] PhysicalError),
    #[error("history reader capability failed: {0}")]
    Retention(#[from] RetentionError),
}

pub struct ReaderFailure {
    delivery: RetainedRole,
    error: NativeLoadError,
}
impl ReaderFailure {
    pub(super) fn new(delivery: RetainedRole, error: NativeLoadError) -> Self {
        Self { delivery, error }
    }
    pub fn error(&self) -> &NativeLoadError {
        &self.error
    }
    pub fn into_parts(self) -> (RetainedRole, NativeLoadError) {
        (self.delivery, self.error)
    }
}

pub enum ReaderOutcome {
    Ready(ManagedParts),
    Failed(ReaderFailure),
}

/// Payload-bearing outcome drops before the actual construction/account pin.
/// No ID lookup can substitute a replacement session's capabilities.
pub struct ReaderCompletion {
    outcome: ReaderOutcome,
    reader: ConstructionRole,
}
impl ReaderCompletion {
    pub(super) fn with_outcome(reader: ConstructionRole, outcome: ReaderOutcome) -> Self {
        Self { outcome, reader }
    }
    pub(super) fn ready(reader: ConstructionRole, delivery: RetainedRole) -> Self {
        Self {
            outcome: ReaderOutcome::Ready(ManagedParts::from_role(delivery)),
            reader,
        }
    }
    pub(super) fn failed(
        reader: ConstructionRole,
        delivery: RetainedRole,
        error: NativeLoadError,
    ) -> Self {
        Self {
            outcome: ReaderOutcome::Failed(ReaderFailure::new(delivery, error)),
            reader,
        }
    }
    pub(super) fn mentions(&mut self, query: &str) -> Result<bool, NativeLoadError> {
        let ReaderOutcome::Ready(parts) = &self.outcome else {
            return Err(RetentionError::FrozenOwner.into());
        };
        let snapshot = parts.snapshot().ok_or(RetentionError::FrozenOwner)?;
        self.reader.begin_phase()?;
        let reader = SearchPhase(&mut self.reader);
        crate::history::live::search::matches(reader.0, snapshot, query)
            .map_err(HistoryCodecError::from)
            .map_err(NativeLoadError::from)
    }
    pub fn outcome(&self) -> &ReaderOutcome {
        &self.outcome
    }
    pub fn into_parts(self) -> (ConstructionRole, ReaderOutcome) {
        (self.reader, self.outcome)
    }
}

struct SearchPhase<'a>(&'a mut ConstructionRole);
impl Drop for SearchPhase<'_> {
    fn drop(&mut self) {
        self.0.close_phase();
    }
}
