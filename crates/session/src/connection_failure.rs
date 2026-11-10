//! Typed native connection failures without importing any transport protocol.
use nocterm_core::SharedCause;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionFailureKind {
    NeedsUnlock,
    Denied,
    Unsupported,
    Backend,
}
#[derive(Clone, Debug, thiserror::Error)]
#[error("{cause}")]
pub struct ConnectionFailure {
    kind: ConnectionFailureKind,
    #[source]
    cause: SharedCause,
}
impl ConnectionFailure {
    pub fn new(
        kind: ConnectionFailureKind,
        error: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            kind,
            cause: SharedCause::new(error),
        }
    }
    pub fn kind(&self) -> ConnectionFailureKind {
        self.kind
    }
}
impl PartialEq for ConnectionFailure {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && std::ptr::eq(self.cause.as_ref(), other.cause.as_ref())
    }
}
impl Eq for ConnectionFailure {}
