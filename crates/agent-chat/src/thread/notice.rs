//! Status text and its error emphasis have one presentation owner.
use crate::runtime::AgentPersistenceError;
use nocterm_ai::storage::AgentStorageError;
use nocterm_session::local_fs::LocalFsError;
use std::{error::Error, io};

pub(super) struct ThreadNotice {
    text: String,
    error: bool,
}
impl Default for ThreadNotice {
    fn default() -> Self {
        Self {
            text: "Ready — send a message to start the agent".into(),
            error: false,
        }
    }
}
impl ThreadNotice {
    pub(super) fn text(&self) -> String {
        self.text.clone()
    }
    pub(super) fn is_error(&self) -> bool {
        self.error
    }
    /// A text-only transition deliberately retains existing error emphasis.
    pub(super) fn present(&mut self, text: String) {
        self.text = text;
    }
    pub(super) fn clear_text(&mut self) {
        self.text.clear();
    }
    pub(super) fn mark_error(&mut self) {
        self.error = true;
    }
    pub(super) fn clear_error(&mut self) {
        self.error = false;
    }
}

pub(super) fn persistence_failure(error: &AgentPersistenceError) -> String {
    // The persistence wrapper is transparent: start at its concrete inner error
    // so a leaf storage failure remains available to the recovery instruction.
    let cause: &(dyn Error + 'static) = match error {
        AgentPersistenceError::Storage(error) => error,
        AgentPersistenceError::Worker(error) => error,
    };
    let recovery = persistence_recovery(cause);
    format!("Could not save chat: {error}. Queued messages are retained; {recovery}")
}

fn persistence_recovery(mut cause: &(dyn Error + 'static)) -> &'static str {
    loop {
        if matches!(
            cause.downcast_ref::<AgentStorageError>(),
            Some(
                AgentStorageError::EnvironmentUnsupported(_)
                    | AgentStorageError::ScopeUnsupported(_)
            )
        ) {
            return "this build lacks the native storage support needed for this chat. Update Nocterm before retrying.";
        }
        if let Some(error) = cause.downcast_ref::<io::Error>() {
            match error.kind() {
                io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => {
                    return "free disk space before retrying.";
                }
                io::ErrorKind::PermissionDenied => {
                    return "allow access to the chat storage directory before retrying.";
                }
                _ => {}
            }
        }
        if matches!(
            cause.downcast_ref::<LocalFsError>(),
            Some(LocalFsError::PermissionDenied(_))
        ) {
            return "allow access to the chat storage directory before retrying.";
        }
        let Some(source) = cause.source() else {
            return "resolve the storage error before retrying.";
        };
        cause = source;
    }
}

#[cfg(test)]
mod tests;
