//! Actual read paths and the first native fault retain original prepaid owners.
use nocterm_ai::{
    model::json::deferred::{DecodeLedger, NativeAllocationError, NativeScratch},
    storage::{AgentStorageError, ManagedFileFailure},
};
use nocterm_core::{
    SharedResidentError,
    fs::{AdmittedFileError, AdmittedPath},
};
#[cfg(test)]
use std::path::PathBuf;
use std::{cell::RefCell, io, path::Path};

pub(super) type PaidPath = AdmittedPath<NativeScratch>;
pub(super) enum HeldPath {
    #[cfg(test)]
    Legacy(PathBuf),
    Managed(PaidPath),
}
impl std::ops::Deref for HeldPath {
    type Target = Path;
    fn deref(&self) -> &Path {
        match self {
            #[cfg(test)]
            Self::Legacy(path) => path,
            Self::Managed(path) => path.as_path(),
        }
    }
}
struct PreparedReadFault {
    path: PaidPath,
    header: NativeScratch,
}
struct FaultState {
    prepared: Option<PreparedReadFault>,
    failure: Option<ManagedFileFailure>,
}
pub(in crate::loading) struct ReadFaultContext {
    state: RefCell<FaultState>,
}
impl ReadFaultContext {
    pub(in crate::loading) fn prepare(
        dir: &Path,
        id: &str,
        ledger: &DecodeLedger,
    ) -> Result<Self, AgentStorageError> {
        crate::writing::native_paths::validate_identity(id, ledger)?;
        let header = NativeScratch::reserve_native(ledger, ManagedFileFailure::allocation_bytes())
            .map_err(AgentStorageError::ManagedAdmission)?;
        let path = crate::writing::native_paths::document(dir, id, ".json", ledger)?;
        Ok(Self {
            state: RefCell::new(FaultState {
                prepared: Some(PreparedReadFault { path, header }),
                failure: None,
            }),
        })
    }
    pub(super) fn capture(&self, error: io::Error) -> ManagedFileFailure {
        let mut state = self.state.borrow_mut();
        if let Some(failure) = &state.failure {
            return failure.clone();
        }
        let prepared = state
            .prepared
            .take()
            .expect("first original native read fault");
        let failure = SharedResidentError::new(
            AdmittedFileError::from_native(prepared.path, error),
            prepared.header,
        );
        state.failure = Some(failure.clone());
        failure
    }
    pub(super) fn failure(&self) -> Option<ManagedFileFailure> {
        self.state.borrow().failure.clone()
    }
}

/// Exact pinned std path conversion scratch, separate from retained paths.
pub(super) fn open_scratch(
    path: &Path,
    ledger: &DecodeLedger,
) -> Result<NativeScratch, NativeAllocationError> {
    #[cfg(unix)]
    let bytes = match path.as_os_str().as_encoded_bytes().len() {
        length if length < 384 => 0,
        length => length
            .checked_add(1)
            .ok_or(NativeAllocationError::Arithmetic)?,
    };
    // Windows' native path conversion remains a separate platform proof gate.
    #[cfg(not(unix))]
    let bytes = 0;
    NativeScratch::reserve_native(ledger, bytes)
}
