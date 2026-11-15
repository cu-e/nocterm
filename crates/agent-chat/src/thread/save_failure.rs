//! Recovery guidance for typed chat-save and history-load failures.
use crate::runtime::{AgentPersistenceError, PersistenceFailure};
use nocterm_ai::storage::AgentStorageError;

pub(super) fn message(error: &AgentPersistenceError) -> String {
    let failure = PersistenceFailure::of(error);
    if let Some(guidance) = history_guidance(failure) {
        return format!("Could not save chat: {guidance} Queued messages are retained.");
    }
    let recovery = match failure {
        PersistenceFailure::Unsupported => {
            "this is a limitation of the history storage: this build lacks the native storage support needed for this chat. Update Nocterm before retrying."
        }
        PersistenceFailure::StorageFull => "free disk space before retrying.",
        PersistenceFailure::AccessDenied => {
            "allow access to the chat storage directory before retrying."
        }
        _ => "resolve the storage error before retrying.",
    };
    format!("Could not save chat: {error}. Queued messages are retained; {recovery}")
}

pub(super) fn load_failure(error: &AgentStorageError) -> String {
    if let Some(guidance) = history_guidance(PersistenceFailure::of_storage(error)) {
        return format!("Could not open saved chat: {guidance}");
    }
    format!("Could not open saved chat: {error}")
}

fn history_guidance(failure: PersistenceFailure) -> Option<&'static str> {
    match failure {
        PersistenceFailure::HistoryMemory => {
            Some("history needs more memory. Close an unused chat and try again.")
        }
        PersistenceFailure::HistoryBusy => {
            Some("history is busy. Close an unused chat or try again shortly.")
        }
        PersistenceFailure::ChatLimit => {
            Some("this conversation is too large. Start a new chat to continue.")
        }
        _ => None,
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

#[cfg(test)]
mod guidance_tests;
