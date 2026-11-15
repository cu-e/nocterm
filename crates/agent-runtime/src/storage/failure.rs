//! Why a chat save failed, read from its typed cause chain for user guidance.
use super::AgentPersistenceError;
use nocterm_ai::{history::HistoryCodecError, storage::AgentStorageError};
use nocterm_session::local_fs::LocalFsError;
use std::error::Error;

/// The class of a failed save. Each class needs different guidance: only a full
/// volume is fixed by freeing space, and an unsupported location is not fixed by
/// retrying at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersistenceFailure {
    /// Retained histories and work have filled the installation's memory allowance.
    HistoryMemory,
    /// Retained or loading histories have filled the shared payload allowance.
    HistoryBusy,
    /// The conversation exceeds its own encoded, structural or row allowance.
    ChatLimit,
    /// The history storage cannot provide the guarantees this chat's location needs.
    Unsupported,
    /// The volume holding the history is out of space or quota.
    StorageFull,
    /// The operating system refused access to the history location.
    AccessDenied,
    /// Any other filesystem, encoding or persistence-worker failure.
    Write,
}
impl PersistenceFailure {
    pub fn of(error: &AgentPersistenceError) -> Self {
        match error {
            AgentPersistenceError::Storage(error) => classify(error),
            AgentPersistenceError::Worker(error) => classify(error),
        }
    }
    pub fn of_storage(error: &AgentStorageError) -> Self {
        classify(error)
    }
}

fn classify(error: &(dyn Error + 'static)) -> PersistenceFailure {
    let mut frame = Some(error);
    let mut denied = false;
    while let Some(current) = frame {
        if let Some(storage) = current.downcast_ref::<AgentStorageError>() {
            match storage {
                AgentStorageError::ResidentCapacity { .. } => {
                    return PersistenceFailure::HistoryMemory;
                }
                AgentStorageError::PayloadCapacity => return PersistenceFailure::HistoryBusy,
                AgentStorageError::Codec(
                    HistoryCodecError::EncodedLimit
                    | HistoryCodecError::StructuralLimit
                    | HistoryCodecError::CollectionLimit,
                ) => return PersistenceFailure::ChatLimit,
                _ => {}
            }
        }
        if matches!(
            current.downcast_ref::<HistoryCodecError>(),
            Some(
                HistoryCodecError::EncodedLimit
                    | HistoryCodecError::StructuralLimit
                    | HistoryCodecError::CollectionLimit
            )
        ) {
            return PersistenceFailure::ChatLimit;
        }
        if let Some(
            AgentStorageError::EnvironmentUnsupported(_) | AgentStorageError::ScopeUnsupported(_),
        ) = current.downcast_ref::<AgentStorageError>()
        {
            return PersistenceFailure::Unsupported;
        }
        if let Some(LocalFsError::PermissionDenied(_)) = current.downcast_ref::<LocalFsError>() {
            denied = true;
        }
        if let Some(io) = current.downcast_ref::<std::io::Error>() {
            match io.kind() {
                std::io::ErrorKind::StorageFull | std::io::ErrorKind::QuotaExceeded => {
                    return PersistenceFailure::StorageFull;
                }
                std::io::ErrorKind::PermissionDenied => denied = true,
                _ => {}
            }
        }
        frame = current.source();
    }
    if denied {
        PersistenceFailure::AccessDenied
    } else {
        PersistenceFailure::Write
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_core::{SharedCause, SharedError};
    use std::{io, sync::Arc};

    fn filesystem(error: io::Error) -> AgentStorageError {
        LocalFsError::from_io("history", error).into()
    }
    fn worker(error: AgentStorageError) -> AgentPersistenceError {
        nocterm_store::StoreError::Backend(SharedCause::new(error)).into()
    }

    #[test]
    fn history_limits_remain_distinct_through_the_writer_cause_chain() {
        use nocterm_ai::history::HistoryCodecError;
        let cases = [
            (
                AgentStorageError::ResidentCapacity {
                    retained: 65_410_245,
                    incoming: 14_188_544,
                    limit: 66_060_288,
                },
                PersistenceFailure::HistoryMemory,
            ),
            (
                AgentStorageError::PayloadCapacity,
                PersistenceFailure::HistoryBusy,
            ),
            (
                HistoryCodecError::EncodedLimit.into(),
                PersistenceFailure::ChatLimit,
            ),
            (
                HistoryCodecError::StructuralLimit.into(),
                PersistenceFailure::ChatLimit,
            ),
            (
                HistoryCodecError::CollectionLimit.into(),
                PersistenceFailure::ChatLimit,
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(PersistenceFailure::of_storage(&error), expected);
            assert_eq!(PersistenceFailure::of(&error.clone().into()), expected);
            assert_eq!(PersistenceFailure::of(&worker(error)), expected);
        }
    }

    #[test]
    fn a_missing_capability_is_unsupported_directly_and_behind_the_writer() {
        let error = AgentStorageError::EnvironmentUnsupported(Arc::new(Default::default()));
        assert_eq!(
            PersistenceFailure::of(&error.clone().into()),
            PersistenceFailure::Unsupported
        );
        assert_eq!(
            PersistenceFailure::of(&worker(error)),
            PersistenceFailure::Unsupported
        );
    }
    #[test]
    fn only_a_full_volume_or_quota_is_storage_full() {
        for kind in [io::ErrorKind::StorageFull, io::ErrorKind::QuotaExceeded] {
            assert_eq!(
                PersistenceFailure::of(&worker(filesystem(io::Error::from(kind)))),
                PersistenceFailure::StorageFull
            );
        }
        let cleanup = AgentStorageError::StageCleanup {
            operation: SharedError::from_error(LocalFsError::from_io(
                "history",
                io::Error::from(io::ErrorKind::StorageFull),
            )),
            cleanup: SharedError::from_error(LocalFsError::Cancelled),
        };
        assert_eq!(
            PersistenceFailure::of(&worker(cleanup)),
            PersistenceFailure::StorageFull
        );
    }
    #[test]
    fn refused_access_and_other_failures_are_distinguished() {
        assert_eq!(
            PersistenceFailure::of(&worker(filesystem(io::Error::from(
                io::ErrorKind::PermissionDenied
            )))),
            PersistenceFailure::AccessDenied
        );
        assert_eq!(
            PersistenceFailure::of(&worker(filesystem(io::Error::other("device error")))),
            PersistenceFailure::Write
        );
        assert_eq!(
            PersistenceFailure::of(&nocterm_store::StoreError::Closing.into()),
            PersistenceFailure::Write
        );
    }
}
