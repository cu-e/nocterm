//! Status text and its error emphasis have one presentation owner.
use crate::runtime::{AgentPersistenceError, PersistenceFailure};
use nocterm_ai::storage::AgentStorageError;

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
    let failure = PersistenceFailure::of(error);
    if let Some(guidance) = history_guidance(failure) {
        return format!("Could not save chat: {guidance} Queued messages are retained.");
    }
    let recovery = match failure {
        PersistenceFailure::Unsupported => {
            "this build lacks the native storage support needed for this chat. Update Nocterm before retrying."
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
mod tests;
