//! The window shell and the seams features plug into.
//!
//! Owns: the permanent outer connection dock and inner project docks,
//! tab membership and cached session context, one authoritative native dock per surface,
//! window menus and chrome, panel groups (the sidebar and the independent side panel),
//! registration of panels and widgets under the ids of the plugin catalog (the shell's own
//! `plugin::{CORE, GROUPS}` and the id queries), the resolved shell layout (`ShellLayout`
//! follows the app-wide `[layouts]` settings of `nocterm-window-layout`, resolves the selected
//! built-in or saved preset against the catalog and writes only the selection; the status bar
//! and the panel groups take their members, order and sides from it on every change),
//! per-surface background membership, and per-column layout projections.
//! Does not own: session connections, feature views, native filesystem implementations,
//! persistence workers, project definitions or connection authentication,
//! or external-program launch.
//! Ports used: `SessionFactory`, `ConnectionDirectory`, `Item`, `Panel`,
//! `LocalTerminal`, `ProjectTerminalOpen`, `ProjectTerminalResolve`,
//! `ChatConnectionOpen`; project terminal contracts/refusals live in workspace-ports
//! without shell reexports. Private placement and typed terminal admission remain here.
//! Layout and the checked plugin catalog arrive as explicit `Deps`.
//! Ports provided: workspace actions, settings pages, host and terminal access,
//! file drag contracts, immutable tab/session and selected monitor-source observations,
//! plus separate state, text,
//! input, live-command, execution, prompt, reconnect and user-paste ports, plus
//! the optional consumer-named ConnectionFactSession original-session receipt.
//! Settings section: reads `[layouts]` (owned by `nocterm-window-layout`) through the injected
//! settings handle; a table that does not read is never overwritten. Globals: none.
//! Host resolution uses an explicitly supplied optional local executor; it owns no runner.
//! Invariants: each surface has one native placement authority; each tab
//! owns its feature handle and subscriptions; cached session identity preserves
//! its filesystem and executor Arcs; local command focus leaves central session
//! context available. Owners commit transitions before the shell emits events
//! and applies focus. Live source receipts survive hiding; deferred UI selection
//! receipts expire after ABA. The window retains all ordinary connection tabs;
//! each project is one outer Item containing its original inner `ProjectSurface`.
//! A bounded weak directory routes original sources without mirroring membership
//! or pane order. UI targets validate the native window and actual outer visibility;
//! window and inner selection generations both expire stale callbacks.
//! `ProjectTerminalOpen` captures an original ordinary connection configuration;
//! folder terminals use normal authentication, filesystem access and reconnect.
//! Nested surfaces follow ADR0067. All application windows share the
//! bootstrapped layout handle.
//! The optional test-support feature shares the original session Item fixture
//! with private connected/generation transitions and supplied filesystem ports.
//! Background promotion retains the same Item entity; close detaches subscriptions
//! before calling the feature's close hook. Local directory synchronization reports typed
//! capability refusals and preserves encoding causes through its LocalTerminal port.
//! SurfaceSelection owns original source, expiring selection receipts and presentation lifecycle.
//! SurfaceState owns initial opening issuance, the starting folder and monitoring metadata.
//! SurfaceContent owns foreground and off-dock membership and returns tab effects;
//! the surface entity applies events and focus. Background promotion transfers the
//! original TabEntry together with its subscriptions into the native dock.
//!
//! Features contribute tabs through [`Item`], sidebar views through [`Panel`],
//! commands through [`Workspace::register_action`], and session opening through
//! [`SessionFactory`]. [`WorkspaceEvent`] reports observable shell changes.

//! Terminal operations report typed refusals and program opening retains launch validation causes.

mod actions;
pub mod command_palette;
mod connection_directory;
mod context_ports;
pub use context_ports::{ChatConnectionOpen, ChatConnectionOpenError};
mod dock_item;
mod file_drag;
pub mod host;
mod item;
mod local_terminal;
mod open_on_host;
mod panel;
pub mod plugin;
mod provider_registry;
pub use provider_registry::ProjectProviderRegistry;
mod tab_groups;
pub use open_on_host::OpenOnHostError;
mod workspace;

pub use actions::*;
pub use connection_directory::ConnectionDirectory;
pub use file_drag::{FileDrag, RemoteFileDrag};
pub use host::{Host, HostKey, ProgramSpec, ShellSyntax};
pub use item::{Item, ItemEvent, ItemHandle, TabState};
pub use local_terminal::{LocalDirectoryError, LocalTerminal, LocalTerminalTarget};
use nocterm_workspace_ports::ContextSelectionError;
pub use panel::{Panel, PanelHandle};
// The terminal and connection contracts live below the shell (ADR 0062).
pub use nocterm_workspace_ports::{
    ConnectionFactSession, ConnectionSummary, ExpectedSession, ExplorerSource, FactSessionSnapshot,
    LiveCommandLease, LiveCommands, ProjectCreationSource, ProjectParentCapture,
    ProjectParentDescriptor, ProjectParentError, RemoteReconnect, SessionContext, SignInPrompt,
    SignInPrompts, TerminalCapabilities, TerminalCapability, TerminalEntry, TerminalError,
    TerminalExecution, TerminalInfo, TerminalInput, TerminalState, TerminalStatus, TerminalText,
    TerminalTextSource, TextRequest, UserPromptToken, UserTerminalPaste,
};
pub use workspace::{
    ContextSelectionReceipt, ContextWindowReceipt, Deps, ExpandablePanel, GeneralTerminalTarget,
    LayoutChoice, LayoutSwitchError, MalformedLayouts, PanelRegistration, PanelRequest,
    ProjectSurface, ProjectSurfaceEvent, ProjectSurfaceFactory, ProjectSurfaceReceipt,
    RegistrationError, SessionFactory, SessionSpec, TabCloseScope, TabPane, TabSnapshot,
    WidgetRegistration, Workspace, WorkspaceEvent, admit_project_terminal,
    validate_project_terminal_target,
};

/// The key context of the workspace, for keymap entries.
pub const KEY_CONTEXT: &str = "Workspace";

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use nocterm_workspace_ports::MonitorSource;
