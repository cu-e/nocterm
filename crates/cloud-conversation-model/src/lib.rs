//! What a desktop window shows of one Cloud chat, and the port it reads and steers it through.
//!
//! Owns: the neutral transcript of a Cloud chat ([`CloudTranscript`] with its [`Row`]s, [`Run`]s,
//! [`Approval`]s and [`PendingCommand`]s) and the rule by which a [`ConversationDelta`] changes
//! it; how far the shown history is confirmed ([`Freshness`]); what the person asks of a chat
//! ([`ConversationAction`], with an explicit model and neutral inline image bytes); the servers a chat is bound to and can be bound to
//! ([`ServerBinding`], [`ServerChoice`], ADR 0147); the port [`CloudConversation`] with its
//! updates ([`ConversationUpdate`]) and typed [`CloudConversationError`].
//!
//! Does not own: the device protocol, verification, reduction of runner events, cryptography or
//! storage (the contract device core inside `nocterm-hub-io` produces the deltas), the GPUI
//! entity or any view (`nocterm-cloud-chat`, `nocterm-agent`), the runner's own domain
//! (`nocterm-cloud-model`) or the agent vocabulary (`nocterm-cloud-agents-model`).
//!
//! Ports provided: [`CloudConversation`], implemented by `nocterm-hub-io` and consumed by
//! `nocterm-cloud-chat`. Ports used: none.
//!
//! Settings: none. Globals: none.
//!
//! Invariants: no I/O and no protocol types; a transcript is display data and grants nothing;
//! a delta that does not continue the held rows resets nothing it does not name; a pending
//! command is the device's own record and never stands for an accepted prompt; freshness is
//! never `Current` without a fresh runner witness.
//!
//! With the `test-support` feature, [`testing::FakeConversation`] implements the port in
//! memory and [`testing::DeltaBuilder`] writes deltas for consumers' tests.

mod action;
mod delta;
mod error;
mod freshness;
mod ids;
mod image;
mod port;
mod servers;
mod transcript;

pub use action::ConversationAction;
pub use delta::ConversationDelta;
pub use error::CloudConversationError;
pub use freshness::Freshness;
pub use ids::{CommandKey, ConversationKey, IntentKey, RunKey};
pub use image::{ImageMime, PromptImage};
pub use port::{
    CloudConversation, ConversationFuture, ConversationUpdate, ConversationUpdates, ProviderOption,
};
pub use servers::{
    BoundServer, DEFAULT_BINDING_WINDOW, MAX_BINDING_WINDOW, ServerAccess, ServerBinding,
    ServerChoice, ServerKey, ServerRefusal,
};
pub use transcript::{
    Approval, CloudTranscript, CommandKind, CommandState, ModelName, PendingCommand, Row, RowRole,
    Run, RunStatus, SessionEnd, SessionRow, SessionState, ToolOutcome, ToolRow, ToolState, Usage,
};

#[cfg(any(test, feature = "test-support"))]
pub mod testing;
