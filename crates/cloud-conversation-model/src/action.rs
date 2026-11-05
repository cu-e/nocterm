//! What the person asks of an open Cloud chat.
use crate::{IntentKey, RunKey, ServerKey};

/// One request. The implementation builds the signed command from the verified transcript; a
/// request names only what the person chose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConversationAction {
    /// A prompt, with the chat's model. With a `server`, it runs against that bound server
    /// ([`crate::ServerBinding::selectable`]); without one, at the runner's default destination.
    Prompt {
        text: String,
        server: Option<ServerKey>,
    },
    /// Stop a run; without one, the active run.
    Stop { run: Option<RunKey> },
    /// Allow a run this device prompted to use its model (ADR 0129).
    Allow { run: RunKey },
    /// Answer a pending tool approval.
    Answer { intent: IntentKey, approve: bool },
}
impl ConversationAction {
    /// The prompt text of a prompt.
    pub fn prompt_text(&self) -> Option<&str> {
        match self {
            Self::Prompt { text, .. } => Some(text),
            _ => None,
        }
    }
}
