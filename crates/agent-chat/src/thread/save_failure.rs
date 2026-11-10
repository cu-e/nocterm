//! Guidance shown when a chat snapshot could not be saved. Queued messages stay
//! queued in every case; only the advice depends on why the save failed.
use crate::runtime::{AgentPersistenceError, PersistenceFailure};

pub(super) fn message(error: &AgentPersistenceError) -> String {
    match PersistenceFailure::of(error) {
        PersistenceFailure::Unsupported => format!(
            "Could not save chat: {error}. This is a limitation of the history storage on this \
             system, not a disk or agent problem. Queued messages are retained and are not sent \
             until the chat can be saved."
        ),
        PersistenceFailure::StorageFull => format!(
            "Could not save chat: the disk holding chat history is full ({error}). Queued \
             messages are retained; free disk space before continuing."
        ),
        PersistenceFailure::AccessDenied => format!(
            "Could not save chat: access to the chat history folder was denied ({error}). Queued \
             messages are retained; check the folder's permissions before continuing."
        ),
        PersistenceFailure::Write => format!(
            "Could not save chat: {error}. Queued messages are retained and are sent once the \
             chat has been saved."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::message;
    use nocterm_ai::storage::AgentStorageError;
    use nocterm_session::local_fs::LocalFsError;
    use std::{io, sync::Arc};

    #[test]
    fn only_a_full_disk_suggests_freeing_space() {
        let unsupported: crate::runtime::AgentPersistenceError =
            AgentStorageError::EnvironmentUnsupported(Arc::new(Default::default())).into();
        let full: crate::runtime::AgentPersistenceError = AgentStorageError::from(
            LocalFsError::from_io("history", io::Error::from(io::ErrorKind::StorageFull)),
        )
        .into();
        let failed: crate::runtime::AgentPersistenceError = AgentStorageError::from(
            LocalFsError::from_io("history", io::Error::other("device error")),
        )
        .into();
        assert!(message(&unsupported).contains("limitation of the history storage"));
        assert!(!message(&unsupported).contains("disk space"));
        assert!(message(&full).contains("free disk space"));
        assert!(!message(&failed).contains("disk space"));
        for text in [message(&unsupported), message(&full), message(&failed)] {
            assert!(text.contains("Queued messages are retained"));
        }
    }
}
