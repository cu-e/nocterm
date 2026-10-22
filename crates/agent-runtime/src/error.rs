//! Typed refusals before allocating any native agent or bridge work.
#[derive(Clone, Debug, thiserror::Error)]
pub enum AgentActivationError {
    #[error("{0}")]
    Storage(#[from] nocterm_ai::storage::AgentStorageError),
    #[error("{0}")]
    Origin(#[from] nocterm_ai::execution_origin::ExecutionOriginError),
    #[error("The chat is no longer open.")]
    ClientClosed,
}

/// Admission failures retain concrete storage causes until the presentation boundary.
#[derive(Clone, Debug, thiserror::Error)]
pub enum ChatOpenError {
    #[error("{0}")]
    Storage(#[from] nocterm_ai::storage::AgentStorageError),
    #[error("The chat is already open in another window: {key:?}")]
    AlreadyOpen {
        key: nocterm_ai::execution_origin::ChatKey,
        window: gpui::EntityId,
    },
    #[error("The chat was deleted: {0:?}")]
    Deleted(nocterm_ai::execution_origin::ChatKey),
    #[error("The chat ownership reservation is stale: {0:?}")]
    StaleClaim(nocterm_ai::execution_origin::ChatKey),
    #[error("Agent history is shutting down or the client is closed.")]
    Closed,
    #[error("Document ownership incarnation is exhausted.")]
    IncarnationExhausted,
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{error::Error as _, sync::Arc};
    #[test]
    fn chat_open_errors_retain_the_original_storage_cause_after_clone() {
        let cause = Arc::new(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        let error = ChatOpenError::from(nocterm_ai::storage::AgentStorageError::Read {
            path: "history".into(),
            source: cause.clone(),
        });
        let cloned = error.clone();
        let stored = cloned
            .source()
            .unwrap()
            .downcast_ref::<nocterm_ai::storage::AgentStorageError>()
            .unwrap();
        let nocterm_ai::storage::AgentStorageError::Read { source, .. } = stored else {
            panic!("original storage classification");
        };
        assert!(Arc::ptr_eq(source, &cause));
    }
}
