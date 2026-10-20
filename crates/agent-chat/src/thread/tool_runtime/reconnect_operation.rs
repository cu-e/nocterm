//! Reconnect completion retires only the receipt this waiter accepted.
use super::super::{
    AgentThread,
    connection_wait::{ConnectionWaitError, ConnectionWaitOperation, WaitTarget},
};
use gpui::{AsyncApp, WeakEntity};
use nocterm_workspace_ports::ExpectedSession;
pub(in crate::thread) struct ReconnectOperation {
    target: WaitTarget,
}
impl ReconnectOperation {
    pub(in crate::thread) fn new(target: WaitTarget) -> Self {
        Self { target }
    }
    pub(in crate::thread) async fn run(
        self,
        thread: &WeakEntity<AgentThread>,
        cx: &mut AsyncApp,
    ) -> Result<(ExpectedSession, serde_json::Value), ConnectionWaitError> {
        let receipt = self.target.expected();
        let result = ConnectionWaitOperation::new(self.target)
            .run(thread, cx)
            .await;
        let _ = thread.update(cx, |thread, _| thread.on_reconnect_wait_finished(receipt));
        result.map(|payload| (receipt, payload))
    }
}
