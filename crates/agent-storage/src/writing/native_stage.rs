//! Actual managed private stage and native failure retain original path credits.
use super::managed_writer::ManagedWire;
use super::{ProducerError, native_paths};
use nocterm_ai::{
    history::{EncodingDecision, HistoryCodecError},
    model::json::deferred::{DecodeLedger, NativeScratch},
    storage::{AgentStorageError, ManagedFileFailure},
};
use nocterm_core::{
    SharedResidentError,
    fs::{AdmittedFileError, AdmittedPublication, AdmittedStageError, PreparedPrivateStage},
};
use std::{io, path::Path};

type Publication = AdmittedPublication<NativeScratch, NativeScratch>;
/// Actual stage is declared before its prepaid final error-header liability.
pub(crate) struct NativePublication {
    publication: Publication,
    #[cfg(unix)]
    _native_scratch: NativeScratch,
    error_header: NativeScratch,
}
impl NativePublication {
    pub(crate) fn publish(self) -> Result<(), AgentStorageError> {
        let Self {
            publication,
            error_header,
            #[cfg(unix)]
            _native_scratch,
        } = self;
        publication.publish().map_err(|error| {
            AgentStorageError::ManagedPublication(SharedResidentError::new(error, error_header))
        })
    }
}
pub(crate) fn stage(
    path: &Path,
    ledger: &DecodeLedger,
    value: &impl ManagedWire,
    decision: EncodingDecision,
) -> Result<NativePublication, AgentStorageError> {
    let error_header = NativeScratch::reserve(ledger, ManagedFileFailure::allocation_bytes())
        .map_err(HistoryCodecError::from)?;
    let parent = native_paths::stage_parent(path, ledger)?;
    // Pinned tempfile permits 65,536 random-name collisions before refusal.
    for attempt in 0..65_536 {
        let destination = native_paths::copy(path, ledger)?;
        let mut leaf = [0u8; 4 + uuid::fmt::Simple::LENGTH];
        leaf[..4].copy_from_slice(b".tmp");
        uuid::Uuid::new_v4().simple().encode_lower(&mut leaf[4..]);
        let leaf = std::str::from_utf8(&leaf).map_err(|_| HistoryCodecError::CandidateMismatch)?;
        let stage = native_paths::join_leaf(parent.as_path(), leaf.as_ref(), ledger)?;
        #[cfg(unix)]
        let native_scratch =
            native_paths::publication_scratch(destination.as_path(), stage.as_path(), ledger)?;
        let (context_credit, wrapper_credit) =
            native_paths::diagnostic_credits(stage.as_path(), ledger)?;
        let prepared = PreparedPrivateStage::new(destination, stage)
            .with_error_credits(context_credit, wrapper_credit);
        match prepared.stage_with(|writer| super::managed_writer::produce(writer, value, decision))
        {
            Ok(publication) => {
                return Ok(NativePublication {
                    publication,
                    #[cfg(unix)]
                    _native_scratch: native_scratch,
                    error_header,
                });
            }
            Err(AdmittedStageError::Native(error)) => {
                if attempt < 65_535
                    && matches!(
                        error.kind(),
                        io::ErrorKind::AlreadyExists | io::ErrorKind::AddrInUse
                    )
                {
                    continue;
                }
                return Err(AgentStorageError::ManagedPublication(
                    SharedResidentError::new(error, error_header),
                ));
            }
            Err(AdmittedStageError::Producer(
                ProducerError::Codec(error),
                destination,
                diagnostics,
            )) => {
                drop(destination);
                drop(diagnostics);
                return Err(error.into());
            }
            Err(AdmittedStageError::Producer(
                ProducerError::Io(source),
                destination,
                diagnostics,
            )) => {
                let (context, wrappers) = match diagnostics {
                    Some(diagnostics) => {
                        let (context, wrappers) = diagnostics.into_parts();
                        (Some(context), Some(wrappers))
                    }
                    None => (None, None),
                };
                let error = AdmittedFileError::from_native_with_context(
                    destination,
                    source,
                    context,
                    wrappers,
                );
                return Err(AgentStorageError::ManagedPublication(
                    SharedResidentError::new(error, error_header),
                ));
            }
        }
    }
    unreachable!("final collision returns its concrete native failure")
}

#[cfg(test)]
mod tests;
