//! Chat-owned command execution contracts, independent of a scheduler or transport.
use crate::{ExecError, ExecExit, ExecRequest, HostExec};
use futures::future::BoxFuture;
use std::sync::Arc;

mod error;
pub use error::JobError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandState {
    Starting,
    Running,
    Exited,
    TimedOut,
    Cancelled,
    Failed,
}
impl CommandState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Exited => "exited",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
    pub fn active(self) -> bool {
        matches!(self, Self::Starting | Self::Running)
    }
}

/// Immutable observation of a command; this data carries no cancellation authority.
#[derive(Clone, Debug)]
pub struct CommandSnapshot {
    pub state: CommandState,
    pub stdout: Vec<u8>,
    pub total_bytes: u64,
    pub exit: Option<ExecExit>,
    pub error: Option<ExecError>,
}
impl Default for CommandSnapshot {
    fn default() -> Self {
        Self {
            state: CommandState::Starting,
            stdout: Vec::new(),
            total_bytes: 0,
            exit: None,
            error: None,
        }
    }
}
impl CommandSnapshot {
    pub fn state(&self) -> CommandState {
        self.state
    }
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }
    pub fn exit(&self) -> Option<&ExecExit> {
        self.exit.as_ref()
    }
    pub fn error(&self) -> Option<&ExecError> {
        self.error.as_ref()
    }
    pub fn active(&self) -> bool {
        self.state.active()
    }
}

/// An admitted command and its runner; the host owns scheduling the returned future.
pub struct CommandStart {
    id: String,
    run: BoxFuture<'static, ()>,
}
impl CommandStart {
    pub fn new(id: String, run: BoxFuture<'static, ()>) -> Self {
        Self { id, run }
    }
    pub fn into_run(self) -> (String, BoxFuture<'static, ()>) {
        (self.id, self.run)
    }
}

/// One chat's bounded commands, retaining the exact executor accepted at admission.
pub trait CommandJobs: Send {
    /// Admit before returning; only polling the returned runner may start the program.
    fn start(
        &mut self,
        terminal: String,
        executor: Arc<dyn HostExec>,
        request: ExecRequest,
        deadline: BoxFuture<'static, ()>,
    ) -> Result<CommandStart, JobError>;
    /// Read retained output only for the command’s originally admitted terminal.
    fn snapshot(&self, id: &str, terminal: &str) -> Result<CommandSnapshot, JobError>;
    /// Cancel the owned command and retain its observable outcome.
    fn cancel(&self, id: &str, terminal: &str) -> Result<CommandSnapshot, JobError>;
    /// Stop this registry’s work without discarding retained observations.
    fn cancel_all(&self);
    /// Whether this registry still owns a starting or running command.
    fn has_active(&self) -> bool;
    /// Cancel jobs whose accepted executor no longer appears under their terminal.
    fn revoke_replaced(&self, current: &[(String, Option<Arc<dyn HostExec>>)]);
}

/// Creates independent registries, with no process or command started at creation.
pub trait CommandJobsFactory: Send + Sync {
    fn create(&self) -> Box<dyn CommandJobs>;
}
