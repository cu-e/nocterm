//! Neutral borrowed transcript presentation, independent of Local/Cloud ownership.
use crate::thread::ThreadState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationPhase {
    Idle,
    Queued,
    Running,
    WaitingApproval,
    NeedsGrant,
    NeedsUnlock,
    NeedsProviderAccess,
    OutcomeUnknown,
    Interrupted,
    Stopped,
    Completed,
    Deleted,
}

/// Borrowed rows use the existing renderer without cloning the transcript.
pub struct ConversationView<'a> {
    pub transcript: &'a ThreadState,
    pub phase: ConversationPhase,
    /// Actual producer activity; the displayed phase alone does not imply it.
    pub generating: bool,
    /// Whether the original producer still accepts its asynchronous updates.
    pub accepts_updates: bool,
    /// Whether this source offers the existing Local fork operation.
    pub can_fork: bool,
    /// Presentation capabilities; native admission still rechecks authority.
    pub can_prompt: bool,
    pub can_stop: bool,
    pub updated_at: u64,
    pub pending_commands: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConversationAction {
    Prompt(String),
    Stop,
}
