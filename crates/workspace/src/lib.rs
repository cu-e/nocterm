//! The window shell, and the seams every feature plugs into.
//!
//! The workspace owns the toolkit dock tree — tabs in resizable central panes,
//! a left sidebar and an independent bottom local terminal — and nothing
//! of what goes in it. Features contribute through four seams:
//!
//! - **Items** ([`Item`]): the content of a tab. A terminal is one; a settings
//!   page is another; an editor or an AI chat would be more.
//! - **Panels** ([`Panel`]): the sidebar's views, switched from the strip of
//!   icons at its foot.
//! - **Actions** ([`Workspace::register_action`]): commands a feature handles
//!   at workspace level, bound to keys by the keymap rather than in code.
//! - **The session opener** ([`Workspace::set_session_opener`]): how a request
//!   to open a session becomes a tab, so the feature that asks (connections)
//!   never depends on the feature that answers (terminal). Programs shown in
//!   a tab of their own ([`Workspace::open_program`]) go the same way.
//!
//! The workspace also reports what the user is looking at
//! ([`WorkspaceEvent`], [`Workspace::active_session`]), so that panels and
//! future integrations follow the active session without knowing who
//! provides it; [`host`] turns that into the machine to run programs on.

mod actions;
pub mod command_palette;
mod connection_directory;
mod dock_item;
pub mod host;
mod item;
mod local_terminal;
mod panel;
mod right_panel;
mod settings_page;
mod tab_groups;
mod terminal_access;
mod workspace;

pub use actions::*;
pub use connection_directory::{ConnectionDirectory, ConnectionSummary};
pub use host::{Host, HostKey, ProgramSpec};
pub use item::{Item, ItemCommand, ItemEvent, ItemHandle, SessionContext, TabState};
pub use local_terminal::LocalTerminal;
pub use panel::{Panel, PanelHandle};
pub use right_panel::{RightPanel, RightPanelEvent};
pub use settings_page::{SettingsPage, SettingsPageHandle, SettingsPageSpec};
pub use terminal_access::{
    SignInPrompt, TerminalAccess, TerminalEntry, TerminalInfo, TerminalStatus, TerminalText,
    TextRequest,
};
pub use workspace::{SessionSpec, TabCloseScope, Workspace, WorkspaceEvent};

/// The key context of the workspace, for keymap entries.
pub const KEY_CONTEXT: &str = "Workspace";
