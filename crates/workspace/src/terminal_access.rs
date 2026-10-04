//! Allowlisted access to existing terminals. No authentication or launch data.
use std::{path::PathBuf, rc::Rc};

use gpui_kit::{App, EntityId, SharedString};
use nocterm_session::Target;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalStatus {
    Connecting,
    AwaitingUser,
    Connected,
    Closed,
}

#[derive(Clone, Debug)]
pub struct TerminalInfo {
    pub title: SharedString,
    pub local: bool,
    pub target: Option<Target>,
    pub profile: Option<SharedString>,
    pub status: TerminalStatus,
    pub cwd: Option<PathBuf>,
    pub at_prompt: Option<bool>,
    pub dirty_input: bool,
    pub alt_screen: bool,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct TextRequest {
    pub max_lines: usize,
    pub max_bytes: usize,
    /// Inclusive cursor: the final line may be returned again after appending.
    pub since_line: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct TerminalText {
    pub text: String,
    pub first_line: u64,
    pub next_line: u64,
    pub truncated: bool,
    pub alt_screen: bool,
}

/// Weak access, checked again for each operation and after user approval.
pub trait TerminalAccess: 'static {
    fn info(&self, cx: &App) -> Option<TerminalInfo>;
    fn read(&self, request: TextRequest, cx: &App) -> Result<TerminalText, String>;
    /// Rejects disconnected/authentication states and reports queue rejection.
    fn send_text(&self, text: &str, cx: &mut App) -> Result<(), String>;
    /// Adds command-specific busy, dirty-input and alternate-screen guards.
    fn run_command(&self, command: &str, cx: &mut App) -> Result<(), String>;
}

#[derive(Clone)]
pub struct TerminalEntry {
    pub item: EntityId,
    pub access: Rc<dyn TerminalAccess>,
    pub title: SharedString,
    pub active: bool,
    pub bottom: bool,
    /// Runs without a tab, opened for an agent.
    pub background: bool,
}
