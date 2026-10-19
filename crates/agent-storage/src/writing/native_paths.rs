//! Managed path assembly admits actual path/cwd storage before allocation.
use nocterm_ai::{
    history::HistoryCodecError,
    model::json::deferred::{DecodeLedger, JsonDecodeError, NativeScratch},
    storage::{AgentStorageError, ManagedPathFailure, ManagedPreparationCause},
};
use nocterm_core::{
    SharedResidentError,
    fs::{
        AdmittedPath, CurrentDirectoryBuffer, CurrentDirectoryError, DirectoryObservation,
        LeafPathPlan, PreparedPathJoin,
    },
};
use std::{ffi::OsStr, path::Path};

type Cause = ManagedPreparationCause;
pub(super) type PaidPath = AdmittedPath<NativeScratch>;

fn path_error(error: nocterm_core::fs::PathAdmissionError) -> Cause {
    CurrentDirectoryError::Admission(error).into()
}
fn reserve(ledger: &DecodeLedger, bytes: usize) -> Result<NativeScratch, Cause> {
    NativeScratch::reserve(ledger, bytes).map_err(Cause::Admission)
}
fn prepared<T>(
    ledger: &DecodeLedger,
    operation: impl FnOnce() -> Result<T, Cause>,
) -> Result<T, AgentStorageError> {
    let header = NativeScratch::reserve_native(ledger, ManagedPathFailure::allocation_bytes())
        .map_err(AgentStorageError::ManagedAdmission)?;
    operation().map_err(|cause| match cause {
        Cause::Admission(JsonDecodeError::Limit { .. }) => {
            HistoryCodecError::StructuralLimit.into()
        }
        cause => AgentStorageError::ManagedPathPreparation(SharedResidentError::new(cause, header)),
    })
}

pub(crate) fn copy(path: &Path, ledger: &DecodeLedger) -> Result<PaidPath, AgentStorageError> {
    prepared(ledger, || {
        let credit = reserve(ledger, AdmittedPath::<NativeScratch>::copy_capacity(path))?;
        AdmittedPath::try_copy(path, credit).map_err(path_error)
    })
}

pub(crate) fn validate_identity(id: &str, ledger: &DecodeLedger) -> Result<(), AgentStorageError> {
    if uuid::Uuid::parse_str(id).is_err() {
        return Err(AgentStorageError::managed_invalid_id(ledger, id)
            .map_err(AgentStorageError::ManagedAdmission)?);
    }
    Ok(())
}

pub(crate) fn document(
    dir: &Path,
    id: &str,
    suffix: &str,
    ledger: &DecodeLedger,
) -> Result<PaidPath, AgentStorageError> {
    validate_identity(id, ledger)?;
    // UUID's longest accepted spelling is URN; preserve its supplied spelling.
    let mut leaf = [0u8; uuid::fmt::Urn::LENGTH + 5];
    let length = id
        .len()
        .checked_add(suffix.len())
        .ok_or(HistoryCodecError::ArithmeticOverflow)?;
    let output = leaf
        .get_mut(..length)
        .ok_or(HistoryCodecError::CandidateMismatch)?;
    output[..id.len()].copy_from_slice(id.as_bytes());
    output[id.len()..].copy_from_slice(suffix.as_bytes());
    let leaf = std::str::from_utf8(output).map_err(|_| HistoryCodecError::CandidateMismatch)?;
    join_leaf(dir, OsStr::new(leaf), ledger)
}

pub(super) fn join_leaf(
    base: &Path,
    leaf: &OsStr,
    ledger: &DecodeLedger,
) -> Result<PaidPath, AgentStorageError> {
    prepared(ledger, || {
        let plan = LeafPathPlan::new(base, leaf).map_err(path_error)?;
        let credit = reserve(ledger, plan.capacity())?;
        plan.allocate(credit).map_err(path_error)
    })
}

pub(super) fn stage_parent(
    destination: &Path,
    ledger: &DecodeLedger,
) -> Result<PaidPath, AgentStorageError> {
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if parent.is_absolute() {
        return copy(parent, ledger);
    }
    prepared(ledger, || {
        let mut capacity = 1;
        loop {
            let credit = reserve(
                ledger,
                CurrentDirectoryBuffer::<NativeScratch>::requested_bytes(capacity)
                    .map_err(path_error)?,
            )?;
            let mut source = CurrentDirectoryBuffer::new(capacity, credit).map_err(path_error)?;
            match source.query().map_err(Cause::Native)? {
                DirectoryObservation::Required(required) => {
                    capacity = required;
                }
                #[cfg(unix)]
                DirectoryObservation::Native(cwd) => return join(cwd, parent, ledger),
                #[cfg(windows)]
                DirectoryObservation::Wide(wide) => {
                    let plan = nocterm_core::fs::WidePathPlan::new(wide).map_err(path_error)?;
                    let retained = reserve(ledger, plan.final_capacity())?;
                    let temporary = reserve(ledger, plan.temporary_bytes())?;
                    let cwd = plan
                        .allocate(wide, retained, &temporary)
                        .map_err(path_error)?;
                    return join(cwd.as_path(), parent, ledger);
                }
            }
        }
    })
}
fn join(base: &Path, right: &Path, ledger: &DecodeLedger) -> Result<PaidPath, Cause> {
    let bytes =
        PreparedPathJoin::<NativeScratch>::scratch_bytes(base, right).map_err(path_error)?;
    let scratch = reserve(ledger, bytes)?;
    let plan = PreparedPathJoin::new(base, right, scratch).map_err(path_error)?;
    let credit = reserve(ledger, plan.capacity().map_err(path_error)?)?;
    plan.allocate(credit).map_err(path_error)
}

pub(super) fn private_directory(
    dir: &Path,
    ledger: &DecodeLedger,
) -> Result<(), AgentStorageError> {
    use nocterm_ai::storage::ManagedFileFailure;
    use nocterm_core::fs::AdmittedFileError;
    let header = NativeScratch::reserve_native(ledger, ManagedFileFailure::allocation_bytes())
        .map_err(AgentStorageError::ManagedAdmission)?;
    let path = copy(dir, ledger)?;
    #[cfg(unix)]
    let _native_scratch = prepared(ledger, || {
        let length = dir.as_os_str().as_encoded_bytes().len();
        let bytes = if length < 384 {
            0
        } else {
            length
                .checked_add(1)
                .ok_or_else(|| path_error(nocterm_core::fs::PathAdmissionError::Arithmetic))?
        };
        reserve(ledger, bytes)
    })?;
    match nocterm_core::paths::ensure_private_dir_checked(path.as_path()) {
        Ok(()) => Ok(()),
        Err(nocterm_core::fs::PrivateDirectoryCheckError::Native(source)) => {
            Err(AgentStorageError::ManagedDirectory(
                SharedResidentError::new(AdmittedFileError::from_native(path, source), header),
            ))
        }
        Err(nocterm_core::fs::PrivateDirectoryCheckError::Policy(reason)) => {
            let failure = prepared(ledger, || {
                let bytes = nocterm_core::fs::DirectoryMessagePlan::new(path.as_path(), reason)
                    .map_err(path_error)?
                    .requested_error_bytes()
                    .map_err(path_error)?;
                let credit = reserve(ledger, bytes)?;
                AdmittedFileError::from_directory_policy(path, reason, credit).map_err(path_error)
            })?;
            Err(AgentStorageError::ManagedDirectory(
                SharedResidentError::new(failure, header),
            ))
        }
    }
}

#[cfg(unix)]
pub(super) fn publication_scratch(
    destination: &Path,
    stage: &Path,
    ledger: &DecodeLedger,
) -> Result<NativeScratch, AgentStorageError> {
    prepared(ledger, || {
        let bytes = nocterm_core::fs::publication_native_scratch_bytes(destination, stage)
            .map_err(path_error)?;
        reserve(ledger, bytes)
    })
}

pub(super) fn diagnostic_credits(
    stage: &Path,
    ledger: &DecodeLedger,
) -> Result<(NativeScratch, NativeScratch), AgentStorageError> {
    prepared(ledger, || {
        let context = reserve(ledger, AdmittedPath::<NativeScratch>::copy_capacity(stage))?;
        let wrappers = tempfile::path_error_allocation_bytes()
            .checked_add(nocterm_core::fs::io_error_custom_allocation_bytes())
            .ok_or_else(|| path_error(nocterm_core::fs::PathAdmissionError::Arithmetic))?;
        Ok((context, reserve(ledger, wrappers)?))
    })
}
