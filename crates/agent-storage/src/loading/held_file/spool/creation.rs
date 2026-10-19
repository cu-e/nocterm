//! Fresh native temp-directory observation and paid original creation failures.
use super::operations::{CreationError, Operations};
use nocterm_ai::{
    model::{
        failure::NativeErrorSlot,
        json::deferred::{DecodeLedger, NativeScratch},
    },
    storage::AgentStorageError,
};
use nocterm_core::fs::{AdmittedPath, EnvironmentBuffer, EnvironmentObservation};
use std::{fmt, fs::File, path::Path};

struct ReadFailure {
    path: AdmittedPath<NativeScratch>,
    cause: CreationError,
}
impl fmt::Debug for ReadFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReadFailure")
            .field("path", &self.path.as_path())
            .field("cause", &self.cause)
            .finish()
    }
}
impl fmt::Display for ReadFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Could not read {}: {}",
            self.path.as_path().display(),
            self.cause
        )
    }
}
impl std::error::Error for ReadFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}
struct PreparedFailure {
    path: AdmittedPath<NativeScratch>,
    slot: NativeErrorSlot<ReadFailure>,
}
impl PreparedFailure {
    fn prepare(path: &Path, ledger: &DecodeLedger) -> Result<Self, AgentStorageError> {
        let slot =
            NativeErrorSlot::prepare(ledger, 0).map_err(AgentStorageError::ManagedAdmission)?;
        let path = crate::writing::native_paths::copy(path, ledger)?;
        Ok(Self { path, slot })
    }
    fn capture(self, cause: CreationError) -> AgentStorageError {
        if let CreationError::Admission(error) = cause {
            return AgentStorageError::ManagedAdmission(error);
        }
        AgentStorageError::ManagedReplayRead(self.slot.capture(ReadFailure {
            path: self.path,
            cause,
        }))
    }
}

pub(super) fn create(path: &Path, ledger: &DecodeLedger) -> Result<File, AgentStorageError> {
    let failure = PreparedFailure::prepare(path, ledger)?;
    let mut operations = Operations::new(ledger);
    let result = tempfile::env::with_temp_dir_override(|overridden| {
        if let Some(directory) = overridden {
            return tempfile::anonymous::tempfile_in(directory, &mut operations);
        }
        let mut source = EnvironmentBuffer::new(0, operations.credit(0)?)?;
        loop {
            match source.query(c"TMPDIR") {
                EnvironmentObservation::Missing => {
                    return tempfile::anonymous::tempfile_in(Path::new("/tmp"), &mut operations);
                }
                EnvironmentObservation::Native(directory) => {
                    return tempfile::anonymous::tempfile_in(Path::new(directory), &mut operations);
                }
                EnvironmentObservation::Required(capacity) => {
                    let replacement =
                        EnvironmentBuffer::new(capacity, operations.credit(capacity)?)?;
                    source = replacement;
                }
            }
        }
    });
    result.map_err(|error| failure.capture(error))
}
