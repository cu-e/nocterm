//! Allowlisted access to existing terminals. No authentication or launch data
//! reaches agents; only the user answers a sign-in prompt.
use crate::{SessionContext, TerminalError};

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use gpui::{App, EntityId, SharedString};
use nocterm_session::{HostExec, Secret, Session, Target};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalStatus {
    Connecting,
    AwaitingUser,
    /// Waiting for a saved credential: unlocking the vault answers the
    /// prompt without anyone typing into the terminal.
    AwaitingVault,
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
    pub expected: ExpectedSession,
    /// The secret the session asks for, which the user can type outside the
    /// terminal. Host key questions stay in the terminal.
    pub sign_in: Option<SignInPrompt>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignInPrompt {
    pub token: UserPromptToken,
    /// What is asked, for example "Password for root@example.com".
    pub label: SharedString,
    /// The previous answer was wrong.
    pub retry: bool,
    /// The answer is hidden while it is typed.
    pub masked: bool,
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

/// Ownership of a live-shell command. User input and reconnect revoke it.
/// Releasing observation never sends a signal; explicit cancellation interrupts
/// only a command whose lease is still active on its original session.
pub struct LiveCommandLease {
    active: Arc<AtomicBool>,
    session: Option<Session>,
}
impl LiveCommandLease {
    pub fn new(active: Arc<AtomicBool>, session: Option<Session>) -> Self {
        Self { active, session }
    }
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
    pub fn cancel(&self) {
        if self.active.swap(false, Ordering::AcqRel)
            && let Some(session) = &self.session
        {
            session.input(vec![3]);
        }
    }
}
impl Drop for LiveCommandLease {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

/// Original connection-facts source, independent of any window projection.
#[derive(Clone)]
pub struct FactSessionSnapshot {
    pub expected: ExpectedSession,
    pub session: SessionContext,
    pub profile: Option<SharedString>,
}
pub trait ConnectionFactSession: 'static {
    fn current(&self, cx: &App) -> Option<FactSessionSnapshot>;
}

/// Reads current state; observation itself grants no session authority.
pub trait TerminalState: 'static {
    fn info(&self, cx: &App) -> Option<TerminalInfo>;
}
pub trait TerminalTextSource: 'static {
    fn read(
        &self,
        expected: ExpectedSession,
        request: TextRequest,
        cx: &App,
    ) -> Result<TerminalText, TerminalError>;
}
pub trait TerminalInput: 'static {
    fn send_text(
        &self,
        expected: ExpectedSession,
        text: &str,
        cx: &mut App,
    ) -> Result<(), TerminalError>;
}
pub trait LiveCommands: 'static {
    fn begin(
        &self,
        expected: ExpectedSession,
        command: &str,
        cx: &mut App,
    ) -> Result<LiveCommandLease, TerminalError>;
}
pub trait TerminalExecution: 'static {
    fn executor(
        &self,
        expected: ExpectedSession,
        cx: &App,
    ) -> Result<Arc<dyn HostExec>, TerminalError>;
}
pub trait SignInPrompts: 'static {
    fn pending(&self, cx: &App) -> Option<SignInPrompt>;
    fn answer(
        &self,
        token: UserPromptToken,
        answer: Secret,
        cx: &mut App,
    ) -> Result<(), TerminalError>;
}
pub trait RemoteReconnect: 'static {
    fn reconnect(
        &self,
        expected: ExpectedSession,
        cx: &mut App,
    ) -> Result<ExpectedSession, TerminalError>;
}
pub trait UserTerminalPaste: 'static {
    fn paste(
        &self,
        expected: ExpectedSession,
        text: &str,
        execute: bool,
        cx: &mut App,
    ) -> Result<(), TerminalError>;
}

/// The original workspace Item and its particular connection, never a screen generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpectedSession {
    item: EntityId,
    epoch: u64,
}
impl ExpectedSession {
    pub fn new(item: EntityId, epoch: u64) -> Self {
        Self { item, epoch }
    }
    pub fn item(self) -> EntityId {
        self.item
    }
    pub fn epoch(self) -> u64 {
        self.epoch
    }
}
/// Binds a user's secret answer to both the connection and the specific prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserPromptToken {
    expected: ExpectedSession,
    prompt_epoch: u64,
}
impl UserPromptToken {
    pub fn new(expected: ExpectedSession, prompt_epoch: u64) -> Self {
        Self {
            expected,
            prompt_epoch,
        }
    }
    pub fn expected(self) -> ExpectedSession {
        self.expected
    }
    pub fn prompt_epoch(self) -> u64 {
        self.prompt_epoch
    }
}

mod capabilities;
pub use capabilities::TerminalCapabilities;

#[derive(Clone)]
pub struct TerminalEntry {
    pub item: EntityId,
    pub capabilities: TerminalCapabilities,
    pub title: SharedString,
    pub active: bool,
    pub bottom: bool,
    /// Runs without a tab, opened for an agent.
    pub background: bool,
}
