//! Agent chats without views: one [`thread::AgentThread`] per chat and the
//! [`roster::ChatRoster`] of a window's selected and retained context chats.
//! Command jobs privately assemble their owned runner; snapshots and sign-in waits expose
//! immutable facts.
//!
//! Owns: AgentThread coordination, private Composer/ChatConfig/ChatDocument owners, private
//! SessionDriver and ToolRuntime owners, the session-scoped ApprovalInbox for provider
//! requests, bridge cards, card tokens and terminal grants, ChatRoster membership and chat
//! lifecycle (new, restart, fork, restore and removal admission).
//! Runtime owns installation-wide history admission. Catalog archives retain an accounted
//! row reference; quiet hidden archives release their entities and references.
//! ChatDeps binds an immutable runtime scope and exact per-chat original seat; every entity factory
//! acquires a scoped document claim and binds the seat before constructing an entity. ChatDocument identity retains its ChatKey.
//! ThreadNotice owns status/error presentation facts; ThreadBindings retains the
//! window/subscriptions. Finite operations return named completions and original-authority
//! polling projections. Archive owns lazy hydration generations, typed pending intentions
//! and delivery residency. BackgroundOpen owns the native-opening handoff and its original
//! workspace receipt.
//! Does not own: views, the workspace shell, ACP transport, connection admission,
//! filesystem storage or image decoding.
//! Ports used: AgentConnector and ToolBridge through the runtime, terminal ports from
//! nocterm-workspace-ports, [`seat::ChatWorkspace`], [`sign_in::SignInTerminal`] and
//! ChatMetadataSource, all supplied with [`chat_deps::ChatDeps`].
//! Ports provided: [`seat::ChatWorkspace`] and [`sign_in::SignInTerminal`] for the window
//! shell to implement.
//! Settings section: reads ai; its schema belongs to nocterm-ai.
//! Globals: none.
//! Invariants: approval cards retain their original chat and stable token; every
//! choice revalidates current session, registration and attachment authority.
//! Archive delivery also validates document ownership and its local load generation;
//! a literal pending draft supersedes the saved draft and its timestamp together.
//! Terminal operation causes stay typed until tool transcripts and sign-in errors are presented.
//! Configuration replies accept only their current session ticket; candidate prompt admission
//! validates bounded payloads before committing draft, image and queue changes.
//! ChatDocument revisions describe accepted durable transitions, not capture or a
//! content fingerprint. Deferred offers remain pending until shared-writer admission;
//! old publication failures cannot pause a newer desired document.
//! Session operations carry their original ticket, connection, session and registration
//! authority. Tool work retains fixed prompt scope, original executor identities and
//! workspace receipts; cancellation preserves readable job outcomes.
//! Joined reconnect calls retain their original busy guards and dispatch in acceptance
//! order after validating original session, registration, workspace and window authority.
//! Ordering releases after synchronous launch; direct calls after connection retain
//! their existing admission path and long-running commands remain independent.
pub mod chat_deps;
pub mod roster;
pub mod seat;
pub mod sign_in;
pub mod thread;

use nocterm_agent_runtime as runtime;
pub use runtime::TerminalAuthRequest;

/// Why a terminal sign-in did not succeed.
#[derive(Debug, Clone, thiserror::Error)]
pub enum TerminalAuthError {
    #[error("Authentication terminal could not be opened.")]
    Opening(#[source] nocterm_core::SharedCause),
    /// The user closed the sign-in terminal.
    #[error("Authentication canceled.")]
    Canceled,
    /// The sign-in command exited unsuccessfully or its terminal closed otherwise.
    #[error("Authentication command did not complete successfully.")]
    Failed,
    /// The agent's sign-in command cannot be launched safely.
    #[error(transparent)]
    InvalidLaunch(#[from] nocterm_session::LaunchValidationError),
}
