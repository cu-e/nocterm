//! Authentication retains its original operation guard until its captured future ends.
use super::{OperationGuard, SessionAuthority};
use nocterm_ai::AgentError;
pub(in crate::thread) struct AuthenticationOperation {
    authority: SessionAuthority,
    guard: OperationGuard,
    auth: futures::future::BoxFuture<'static, Result<(), AgentError>>,
}
impl AuthenticationOperation {
    pub(in crate::thread) fn new(
        authority: SessionAuthority,
        guard: OperationGuard,
        auth: futures::future::BoxFuture<'static, Result<(), AgentError>>,
    ) -> Self {
        Self {
            authority,
            guard,
            auth,
        }
    }
    pub(in crate::thread) async fn run(self) -> (SessionAuthority, Result<(), AgentError>) {
        let _guard = self.guard;
        (self.authority, self.auth.await)
    }
}
