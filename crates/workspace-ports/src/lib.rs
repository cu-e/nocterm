//! What a consumer can do with a terminal or a saved connection, and under
//! which authority, independent of the window shell that hosts them.
//!
//! Owns: the terminal contracts of ADR 0032 (state, text, input, live
//! commands, execution, sign-in prompts, reconnect, user paste), the
//! `TerminalCapabilities` bundle a provider advertises, `ExpectedSession` and
//! `UserPromptToken` authority, `LiveCommandLease`, connection facts,
//! `SessionContext`, `ConnectionSummary` and the typed `TerminalError` and
//! `TerminalCapability` refusals, inert original folder-source metadata and
//! ordinary Explorer session projections and the private immutable MonitorSource.
//! Monitor observations retain the selected inner executor and inert resource identity.
//! Creation sources retain original
//! executor/filesystem capabilities without authorizing a replacement session.
//! Explicit local creation may retain an injected executor without any terminal.
//! Does not own: the workspace, its tabs or items, `ConnectionDirectory`
//! (which opens connections into a workspace), or any implementation.
//! Ports provided: the contracts above, implemented by the terminal feature
//! and consumed by agents, snippets and projects. Ports used: `nocterm-session`
//! and checked folder identity/path values.
//! Settings section: none. Globals: none.
//! Invariants: observation grants no session authority; every operation names
//! its `ExpectedSession` and is refused once that connection changed; terminal-free
//! creation is local only and grants no session authority; a
//! sign-in answer is bound to its prompt; no authentication or launch data
//! reaches consumers.
//! Folder source capture is a human creation seam. ExplorerSource supplies
//! ordinary session filesystems and an initial directory for navigation.
mod connections;
mod errors;
mod explorer;
mod monitor;
mod project_parent;
mod session;
mod terminal;

#[cfg(feature = "test-support")]
pub mod testing;

pub use connections::ConnectionSummary;
pub use errors::{TerminalCapability, TerminalError};
pub use explorer::ExplorerSource;
pub use monitor::MonitorSource;
pub use project_parent::{
    ProjectCreationSource, ProjectParentCapture, ProjectParentDescriptor, ProjectParentError,
};
pub use session::SessionContext;
pub use terminal::{
    ConnectionFactSession, ExpectedSession, FactSessionSnapshot, LiveCommandLease, LiveCommands,
    RemoteReconnect, SignInPrompt, SignInPrompts, TerminalCapabilities, TerminalEntry,
    TerminalExecution, TerminalInfo, TerminalInput, TerminalState, TerminalStatus, TerminalText,
    TerminalTextSource, TextRequest, UserPromptToken, UserTerminalPaste,
};
