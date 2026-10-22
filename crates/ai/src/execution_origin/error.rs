#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExecutionOriginError {
    #[error("Execution origin belongs to {actual:?}, but the consumer expects {expected:?}")]
    ScopeMismatch {
        expected: crate::history::ContextId,
        actual: crate::history::ContextId,
    },
}
