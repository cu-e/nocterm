//! Narrow collaborators for native tool consumers; no execution or persistence here.
use super::{ScopedToolRequest, ToolExecutionError};
use crate::{authority::SessionReceipt, identity::Timestamp, run::DispatchPermit};
use futures::future::BoxFuture;
use nocterm_session::Session;
use std::time::Duration;

/// Current authority, supplied by the coordinator and its retained writer owner.
/// A permit is descriptive: this check must reject changed arguments, revocation
/// and stale writer/session/grant coordinates. The durable ledger and executor
/// once claim separately fence replay; repeated checks do not consume a permit.
/// It does not itself append or acknowledge a durable transition.
pub trait ToolAuthority: Send + Sync {
    fn check(&self, request: &ScopedToolRequest) -> Result<(), ToolExecutionError>;
}

/// Millisecond observation and bounded waits supplied by native composition.
/// Waiting never renews authority; effect admission rechecks after every wait.
pub trait ToolTimer: Send + Sync {
    fn now(&self) -> Timestamp;
    fn delay(&self, duration: Duration) -> BoxFuture<'static, ()>;
}

/// A native session established for its exact admitted receipt.
pub struct OpenedToolSession {
    receipt: SessionReceipt,
    session: Session,
}
impl OpenedToolSession {
    pub fn new(receipt: SessionReceipt, session: Session) -> Self {
        Self { receipt, session }
    }
    pub fn into_parts(self) -> (SessionReceipt, Session) {
        (self.receipt, self.session)
    }
}

/// Opens only the exact already admitted target and selected authentication.
/// Its implementation repeats current authority checks at native connection I/O;
/// profile names, model-chosen addresses and ambient credentials are not fallbacks.
pub trait ScopedSessionOpener: Send + Sync {
    fn open(
        &self,
        request: ScopedToolRequest,
    ) -> BoxFuture<'static, Result<OpenedToolSession, ToolExecutionError>>;
}

/// Background job linkage to its original already committed effect.
/// Starting or returning a command handle does not resolve the mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackgroundCommand {
    dispatch: DispatchPermit,
    terminal: String,
    command: String,
}
impl BackgroundCommand {
    pub fn new(
        dispatch: DispatchPermit,
        terminal: String,
        command: String,
    ) -> Result<Self, ToolExecutionError> {
        if [&terminal, &command]
            .into_iter()
            .any(|id| id.is_empty() || id.len() > 128 || id.contains('\0'))
        {
            return Err(ToolExecutionError::ArgumentMismatch);
        }
        Ok(Self {
            dispatch,
            terminal,
            command,
        })
    }
    pub fn dispatch(&self) -> &DispatchPermit {
        &self.dispatch
    }
    pub fn terminal(&self) -> &str {
        &self.terminal
    }
    pub fn command(&self) -> &str {
        &self.command
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundCommandOutcome {
    /// An actually observed remote program exit, including a nonzero status.
    ObservedExit(u32),
    /// Cancellation, timeout, dropped startup/worker, missing exit or lost connection.
    Unknown,
}

/// Extends the original intent's durable lifecycle, never a second command ledger.
/// Implementations validate its actual stored dispatch and current writer fence.
pub trait BackgroundCommandJournal: Send + Sync {
    /// Atomic idempotent linkage, acknowledged before the job runner's first poll.
    /// Changed reuse conflicts. A storage failure forbids starting the job.
    fn retain(
        &self,
        command: BackgroundCommand,
    ) -> BoxFuture<'static, Result<(), ToolExecutionError>>;
    /// Acknowledged observation for this exact original job only. Unknown outcomes
    /// retain the target's unresolved fence across run completion and restart.
    fn finish(
        &self,
        command: BackgroundCommand,
        outcome: BackgroundCommandOutcome,
    ) -> BoxFuture<'static, Result<(), ToolExecutionError>>;
}

/// Retains native background work independently of an individual tool read waiter.
/// Accepted work is supervised through completion or explicit shutdown; failures
/// are surfaced to the coordinator, never detached and silently discarded.
pub trait ToolTaskSpawner: Send + Sync {
    fn spawn(
        &self,
        task: BoxFuture<'static, Result<(), ToolExecutionError>>,
    ) -> Result<(), ToolExecutionError>;
}

/// Original admitted-job liveness, independent of foreground view completion.
/// Checks original source/session/grant expiry and revocation; it never permits
/// new dispatch, input, reconnect or a replacement execution under another run.
pub trait BackgroundCommandAuthority: Send + Sync {
    fn check(&self, command: &BackgroundCommand) -> Result<(), ToolExecutionError>;
}

/// Cleanup capability for an already retained original effect, including after
/// foreground retirement or writer replacement. The implementation owns current
/// cleanup writer authority, verifies actual original retention and exact IDs,
/// and cannot create a new Retained/dispatch. Unknown keeps its original fence.
/// A real terminal fact already observed before revocation may be acknowledged
/// without renewing target I/O. No value constructor proves such observation.
pub trait BackgroundCommandCleanup: Send + Sync {
    fn finish(
        &self,
        command: BackgroundCommand,
        outcome: BackgroundCommandOutcome,
    ) -> BoxFuture<'static, Result<(), ToolExecutionError>>;
}

/// Host-supplied fresh opaque handles for immutable per-run terminal views.
/// Allocation conveys no connection, execution or source authority.
pub trait ToolSessionHandleSource: Send + Sync {
    fn next(
        &self,
        run: &crate::authority::RunBinding,
    ) -> Result<crate::identity::RemoteSessionId, ToolExecutionError>;
}
