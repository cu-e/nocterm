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
    pub call: &'a TerminalCall,
    pub display: Option<&'a ToolDisplay>,
    pub target: &'a str,
    pub principal: Option<&'a str>,
    pub expires_ms: Option<u64>,
    pub reusable: bool,
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
