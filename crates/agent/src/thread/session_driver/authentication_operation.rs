//! Authentication retains its original operation guard until its captured future ends.
use super::{OperationGuard, SessionAuthority};
use nocterm_ai::history::source::ManagedCommandError;

#[derive(Debug, thiserror::Error)]
pub(in crate::thread) enum AuthenticationError {
    #[error(transparent)]
    Command(#[from] ManagedCommandError),
    #[error("Sign-in cancelled")]
    Cancelled,
    #[error("{0}")]
    Terminal(String),
}
pub(in crate::thread) struct AuthenticationOperation {
    authority: SessionAuthority,
    guard: OperationGuard,
    auth: futures::future::BoxFuture<'static, Result<(), AuthenticationError>>,
}
impl AuthenticationOperation {
    pub(in crate::thread) fn new(
        authority: SessionAuthority,
        guard: OperationGuard,
        auth: futures::future::BoxFuture<'static, Result<(), AuthenticationError>>,
    ) -> Self {
        Self {
            authority,
            guard,
            auth,
        }
    }
    pub(in crate::thread) async fn run(
        self,
    ) -> (SessionAuthority, Result<(), AuthenticationError>) {
        let _guard = self.guard;
        (self.authority, self.auth.await)
    }
}
