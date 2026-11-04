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
    /// The run waits for the person to allow model use (Cloud chats; ADR 0129).
    pub can_allow: bool,
    pub updated_at: u64,
    pub pending_commands: usize,
    /// The source lost contact with its producer and keeps trying; what it shows may be
    /// behind until it reconnects.
    pub reconnecting: bool,
    /// How far the shown history is confirmed by its producer.
    pub freshness: ConversationFreshness,
    /// What the composer offers besides prompts.
    pub composer: ComposerCapabilities,
    /// Prompts the person sent that the producer has not accepted yet, oldest first. They are
    /// shown after the transcript until the accepted prompt replaces them.
    pub pending_prompts: &'a [PendingPrompt<'a>],
}

/// How far a conversation's shown history is confirmed by its producer. A local producer is
/// always live; a Cloud chat is live only after its runner confirmed the head it shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConversationFreshness {
    /// Confirmed by the producer, or produced here.
    #[default]
    Live,
    /// A verified prefix of stored history: the producer did not answer (a sleeping seat), so
    /// newer history may follow. `can_wake` says the source offers to wake it.
    Unconfirmed { can_wake: bool },
    /// The producer confirmed newer history than the hub delivered: the hub withholds it.
    Withheld,
}

/// What a conversation's composer offers besides prompts; a source without any takes text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ComposerCapabilities {
    /// Terminals, servers and images can be attached to a prompt.
    pub attach: bool,
    /// The model, mode and other options of the session can be chosen.
    pub configure: bool,
    /// The source knows the context usage the composer's usage ring shows.
    pub usage: bool,
}

/// Presentation identity of one pending prompt within its source; it confers no authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PendingPromptKey([u8; 16]);
impl PendingPromptKey {
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PendingPromptState {
    /// On its way to the host.
    Sending,
    /// The host holds it and the agent has not accepted it yet.
    Delivered,
    /// Nobody answered its delivery before it expired; retrying sends its text again as a new
    /// prompt.
    Unreachable,
    /// The agent refused it; retrying sends its text again as a new prompt.
    Refused,
}
impl PendingPromptState {
    /// Whether the prompt is still on its way, rather than failed.
    pub fn on_its_way(self) -> bool {
        matches!(self, Self::Sending | Self::Delivered)
    }
}

/// One prompt the person sent, as its source holds it until the agent accepts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingPrompt<'a> {
    pub key: PendingPromptKey,
    pub text: &'a str,
    pub state: PendingPromptState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConversationAction {
    Prompt(String),
    Stop,
    /// Allows the waiting run to use the model within the chat's configured bounds.
    Allow,
}
