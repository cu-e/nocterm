//! Prompt completion carries the exact turn authority that launched its request.
use super::SessionAuthority;
use nocterm_ai::{
    history::source::{ManagedCommandError, ManagedDispatchFuture},
    model::{native::NativeOwned, session::PromptOutcome},
};
pub(in crate::thread) struct PromptOperation {
    authority: SessionAuthority,
    work: ManagedDispatchFuture<Result<NativeOwned<PromptOutcome>, ManagedCommandError>>,
}
impl PromptOperation {
    pub(in crate::thread) fn new(
        authority: SessionAuthority,
        work: ManagedDispatchFuture<Result<NativeOwned<PromptOutcome>, ManagedCommandError>>,
    ) -> Self {
        Self { authority, work }
    }
    pub(in crate::thread) async fn run(
        self,
    ) -> (
        SessionAuthority,
        Result<NativeOwned<PromptOutcome>, ManagedCommandError>,
    ) {
        (self.authority, self.work.await)
    }
}
