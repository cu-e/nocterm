//! The window shell and the seams features plug into.
//!
//! Owns: tab membership and cached session context, native dock placement,
//! window menus and chrome, sidebar and independent side-panel visibility,
//! background-session membership, and per-column layout projections.
//! Does not own: session connections, feature views, filesystem capabilities,
//! persistence workers, or external-program launch.
//! Ports used: `SessionFactory`, `ConnectionDirectory`, `Item`, `Panel`,
//! `RightPanel`, `LocalTerminal`; layout arrives as an explicit `LayoutHandle`.
//! Ports provided: workspace actions, settings pages, host and terminal access,
//! file drag contracts, immutable tab/session observations and separate state, text,
//! input, live-command, execution, prompt, reconnect and user-paste ports, plus
//! the optional consumer-named ConnectionFactSession original-session receipt.
//! Settings section: none. Globals: none.
//! Host resolution uses an explicitly supplied optional local executor; it owns no runner.
//! Invariants: one native dock is the placement authority; each registered tab
//! owns its feature handle and subscriptions; cached session identity preserves
//! its filesystem and executor Arcs; local command focus leaves central session
//! context available. Owners commit transitions before the shell emits events
//! and applies focus. All application windows share the bootstrapped layout handle.
//! The optional test-support feature shares the original session Item fixture
//! with private connected/generation transitions and supplied filesystem ports.
//! Background promotion retains the same Item entity; close detaches subscriptions
//! before calling the feature's close hook. Local directory synchronization reports typed
//! capability refusals and preserves encoding causes through its LocalTerminal port.
//!
//! Features contribute tabs through [`Item`], sidebar views through [`Panel`],
//! commands through [`Workspace::register_action`], and session opening through
//! [`SessionFactory`]. [`WorkspaceEvent`] reports observable shell changes.

//! Terminal operations report typed refusals and program opening retains launch validation causes.

mod actions;
pub mod command_palette;
mod connection_directory;
mod dock_item;
mod file_drag;
pub mod host;
mod item;
mod local_terminal;
mod open_on_host;
mod panel;
mod right_panel;
mod tab_groups;
pub use open_on_host::OpenOnHostError;
mod workspace;

pub use actions::*;
pub use connection_directory::ConnectionDirectory;
pub use file_drag::{FileDrag, RemoteFileDrag};
pub use host::{Host, HostKey, ProgramSpec, ShellSyntax};
pub use item::{Item, ItemEvent, ItemHandle, TabState};
pub use local_terminal::{LocalDirectoryError, LocalTerminal, LocalTerminalTarget};
pub use panel::{Panel, PanelHandle};
pub use right_panel::{RightPanel, RightPanelEvent};
// The terminal and connection contracts live below the shell (ADR 0062).
pub use nocterm_workspace_ports::{
    ConnectionFactSession, ConnectionSummary, ExpectedSession, FactSessionSnapshot,
    LiveCommandLease, LiveCommands, RemoteReconnect, SessionContext, SignInPrompt, SignInPrompts,
    TerminalCapabilities, TerminalCapability, TerminalEntry, TerminalError, TerminalExecution,
    TerminalInfo, TerminalInput, TerminalState, TerminalStatus, TerminalText, TerminalTextSource,
    TextRequest, UserPromptToken, UserTerminalPaste,
};
pub use workspace::{
    Deps, SessionFactory, SessionSpec, TabCloseScope, TabPane, TabSnapshot, Workspace,
    WorkspaceEvent,
};

/// The key context of the workspace, for keymap entries.
pub const KEY_CONTEXT: &str = "Workspace";

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
