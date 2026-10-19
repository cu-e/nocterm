//! A cancellation deadline remains bound to its original connection and turn.
use super::SessionAuthority;
use std::time::Duration;
pub(in crate::thread) struct CancelOperation {
    authority: SessionAuthority,
}
impl CancelOperation {
    pub(in crate::thread) fn new(authority: SessionAuthority) -> Self {
        Self { authority }
    }
    pub(in crate::thread) async fn run(
        self,
        executor: gpui_kit::BackgroundExecutor,
    ) -> SessionAuthority {
        executor.timer(Duration::from_secs(10)).await;
        self.authority
    }
}
