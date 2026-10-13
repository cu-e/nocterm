//! Approval and reconnect authority stays tied to the captured Item connection.
use nocterm_ai::BridgeCall;
use nocterm_workspace::ExpectedSession;
pub(super) struct BoundToolCall {
    call: BridgeCall,
    expected: Option<ExpectedSession>,
}
impl BoundToolCall {
    pub(super) fn new(call: BridgeCall, expected: Option<ExpectedSession>) -> Self {
        Self { call, expected }
    }
    pub(super) fn call(&self) -> &BridgeCall {
        &self.call
    }
    pub(super) fn expected(&self) -> Option<ExpectedSession> {
        self.expected
    }
    pub(super) fn into_call(self) -> BridgeCall {
        self.call
    }
    pub(super) fn reconnected(mut self, receipt: ExpectedSession) -> Self {
        self.expected = Some(receipt);
        self
    }
}
