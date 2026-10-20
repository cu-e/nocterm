//! What a consumer can do with a terminal or a saved connection, and under
//! which authority, independent of the window shell that hosts them.
//!
//! Owns: the terminal contracts of ADR 0032 (state, text, input, live
//! commands, execution, sign-in prompts, reconnect, user paste), the
//! `TerminalCapabilities` bundle a provider advertises, `ExpectedSession` and
//! `UserPromptToken` authority, `LiveCommandLease`, connection facts,
//! `SessionContext`, `ConnectionSummary` and the typed `TerminalError` and
//! `TerminalCapability` refusals.
//! Does not own: the workspace, its tabs or items, `ConnectionDirectory`
//! (which opens connections into a workspace), or any implementation.
//! Ports provided: the contracts above, implemented by the terminal feature
//! and consumed by agents and snippets. Ports used: `nocterm-session`.
//! Settings section: none. Globals: none.
//! Invariants: observation grants no session authority; every operation names
//! its `ExpectedSession` and is refused once that connection changed; a
//! sign-in answer is bound to its prompt; no authentication or launch data
//! reaches consumers.
mod connections;
mod errors;
mod session;
mod terminal;

pub use connections::ConnectionSummary;
pub use errors::{TerminalCapability, TerminalError};
pub use session::SessionContext;
pub use terminal::{
    ConnectionFactSession, ExpectedSession, FactSessionSnapshot, LiveCommandLease, LiveCommands,
    RemoteReconnect, SignInPrompt, SignInPrompts, TerminalCapabilities, TerminalEntry,
    TerminalExecution, TerminalInfo, TerminalInput, TerminalState, TerminalStatus, TerminalText,
    TerminalTextSource, TextRequest, UserPromptToken, UserTerminalPaste,
};
