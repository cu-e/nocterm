//! Approval and reconnect authority stays tied to the captured Item connection.
use nocterm_ai::BridgeCall;
use nocterm_workspace_ports::ExpectedSession;
pub(super) struct BoundToolCall {
    call: BridgeCall,
    expected: Option<ExpectedSession>,
    /// The session a transfer reads from; other calls have none.
    source: Option<ExpectedSession>,
}
impl BoundToolCall {
    pub(super) fn new(call: BridgeCall, expected: Option<ExpectedSession>) -> Self {
        Self {
            call,
            expected,
            source: None,
        }
    }
    pub(super) fn with_source(mut self, source: Option<ExpectedSession>) -> Self {
        self.source = source;
        self
    }
    pub(super) fn source(&self) -> Option<ExpectedSession> {
        self.source
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
