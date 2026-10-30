//! A captured original tool card, independent from a Local token or Cloud IDs.
use super::ConversationError;
use gpui::{App, Task};
use nocterm_ai::{TerminalCall, tool_display::ToolDisplay};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolApprovalChoice {
    Deny,
    AllowOnce,
    AllowForSession,
}
pub struct ToolApprovalView<'a> {
    call: &'a TerminalCall,
    display: Option<&'a ToolDisplay>,
    target: &'a str,
    principal: Option<&'a str>,
    expires_ms: Option<u64>,
    reusable: bool,
}
impl<'a> ToolApprovalView<'a> {
    pub fn new(call: &'a TerminalCall, target: &'a str, reusable: bool) -> Self {
        Self {
            call,
            display: None,
            target,
            principal: None,
            expires_ms: None,
            reusable,
        }
    }
    pub fn with_display(mut self, display: &'a ToolDisplay) -> Self {
        self.display = Some(display);
        self
    }
    pub fn with_principal(mut self, principal: &'a str) -> Self {
        self.principal = Some(principal);
        self
    }
    pub fn with_expiry(mut self, expires_ms: Option<u64>) -> Self {
        self.expires_ms = expires_ms;
        self
    }
    pub fn call(&self) -> &'a TerminalCall {
        self.call
    }
    pub fn display(&self) -> Option<&'a ToolDisplay> {
        self.display
    }
    pub fn target(&self) -> &'a str {
        self.target
    }
    pub fn principal(&self) -> Option<&'a str> {
        self.principal
    }
    pub fn expires_ms(&self) -> Option<u64> {
        self.expires_ms
    }
    pub fn reusable(&self) -> bool {
        self.reusable
    }
}
pub trait ConversationToolApproval {
    fn with_view(
        &self,
        cx: &App,
        visitor: &mut dyn for<'a> FnMut(ToolApprovalView<'a>),
    ) -> Result<(), ConversationError>;
    fn choose(
        &self,
        choice: ToolApprovalChoice,
        cx: &mut App,
    ) -> Task<Result<(), ConversationError>>;
}
