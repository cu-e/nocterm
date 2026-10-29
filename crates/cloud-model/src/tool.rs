//! Scoped existing-terminal-tool requests; no raw transport capability escapes.
use crate::{
    authority::AuthorityError,
    identity::{Fingerprint, IntentId},
    payload::{BoundedJson, PayloadError},
    run::DispatchPermit,
};
use futures::future::BoxFuture;
use nocterm_ai::{TerminalCall, TerminalCallError, tool_display::ToolDisplay};
use nocterm_core::SharedCause;
use std::{fmt, sync::Arc};

mod native;
mod runtime;
pub use native::{
    BackgroundCommand, BackgroundCommandAuthority, BackgroundCommandCleanup,
    BackgroundCommandJournal, BackgroundCommandOutcome, OpenedToolSession, ScopedSessionOpener,
    ToolAuthority, ToolSessionHandleSource, ToolTaskSpawner, ToolTimer,
};
pub use runtime::{ArgumentFingerprint, RunRetirement, RunToolRuntime, RunToolRuntimeFactory};

#[derive(Clone)]
pub struct ScopedToolRequest {
    permit: DispatchPermit,
    call: TerminalCall,
}
impl ScopedToolRequest {
    /// The host supplies its independently checked final-argument digest.
    /// The backend rechecks that digest and original authority before any I/O.
    pub fn new(
        permit: DispatchPermit,
        call: TerminalCall,
        arguments: Fingerprint,
    ) -> Result<Self, ToolExecutionError> {
        call.validate()?;
        if arguments != permit.intent().arguments() || call.writes() != permit.intent().mutates() {
            return Err(ToolExecutionError::ArgumentMismatch);
        }
        Ok(Self { permit, call })
    }
    pub fn permit(&self) -> &DispatchPermit {
        &self.permit
    }
    pub fn call(&self) -> &TerminalCall {
        &self.call
    }
}
impl fmt::Debug for ScopedToolRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopedToolRequest")
            .field("intent", &self.permit.intent().id())
            .field("tool", &self.call.name())
            .finish()
    }
}

/// Remote effect classification supplied only by the trusted native backend.
/// A returned tool observation is not necessarily a finished remote mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolEffectStatus {
    Observed,
    Background(Arc<BackgroundCommand>),
    Unknown,
}

#[derive(Clone)]
pub struct ToolExecutionResult {
    intent: IntentId,
    output: BoundedJson,
    display: ToolDisplay,
    effect: ToolEffectStatus,
}
impl ToolExecutionResult {
    /// The injected native backend supplies the actually accepted destination.
    /// Model/provider metadata never produces this result through the engine port.
    pub fn completed(
        request: &ScopedToolRequest,
        output: BoundedJson,
        destination: Option<String>,
    ) -> Self {
        let mut display = ToolDisplay::new(request.call(), destination);
        display.finish(Ok(output.value().clone()));
        Self {
            intent: request.permit.intent().id(),
            output,
            display,
            effect: ToolEffectStatus::Observed,
        }
    }
    pub fn with_effect_status(
        mut self,
        request: &ScopedToolRequest,
        effect: ToolEffectStatus,
    ) -> Result<Self, ToolExecutionError> {
        if self.intent != request.permit().intent().id() {
            return Err(ToolExecutionError::ArgumentMismatch);
        }
        if let ToolEffectStatus::Background(command) = &effect
            && (command.dispatch() != request.permit()
                || command.dispatch().intent().id() != self.intent
                || !matches!(request.call(), TerminalCall::ExecCommand(_))
                || Some(command.terminal()) != request.call().terminal_id())
        {
            return Err(ToolExecutionError::ArgumentMismatch);
        }
        self.effect = effect;
        Ok(self)
    }
    pub fn effect_status(&self) -> &ToolEffectStatus {
        &self.effect
    }
    pub fn intent(&self) -> IntentId {
        self.intent
    }
    pub fn output(&self) -> &BoundedJson {
        &self.output
    }
    pub fn display(&self) -> &ToolDisplay {
        &self.display
    }
}
impl fmt::Debug for ToolExecutionResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolExecutionResult")
            .field("intent", &self.intent)
            .field("bytes", &self.output.encoded_bytes())
            .finish()
    }
}

pub trait ScopedToolExecutor: Send + Sync {
    /// Validate the full original grant/session/writer coordinates and exact
    /// arguments at effect admission. No ambient key/SSH/shell fallback is allowed.
    fn execute(
        &self,
        request: ScopedToolRequest,
    ) -> BoxFuture<'static, Result<ToolExecutionResult, ToolExecutionError>>;
}

#[derive(Clone, thiserror::Error)]
pub enum ToolExecutionError {
    #[error("the tool arguments are invalid")]
    InvalidArguments(#[from] TerminalCallError),
    #[error("the approved arguments changed before execution")]
    ArgumentMismatch,
    #[error("the original execution authority does not match")]
    Authority(#[from] AuthorityError),
    #[error("the original session is stale")]
    StaleSession,
    #[error("this tool capability is not supported")]
    UnsupportedCapability,
    #[error("the task grant is revoked")]
    Revoked,
    #[error("the task grant has expired")]
    Expired,
    #[error("the target requires a new explicit authentication")]
    NeedsUnlock,
    #[error("the tool execution capacity is full")]
    Capacity,
    #[error("the remote operation may have run; reconciliation is required")]
    OutcomeUnknown,
    #[error("the native tool operation failed")]
    Execution(#[source] SharedCause),
    #[error("the tool result exceeded its bounded format")]
    Payload(#[from] PayloadError),
}
impl fmt::Debug for ToolExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidArguments(_) => "InvalidArguments",
            Self::ArgumentMismatch => "ArgumentMismatch",
            Self::Authority(_) => "Authority",
            Self::StaleSession => "StaleSession",
            Self::UnsupportedCapability => "UnsupportedCapability",
            Self::Revoked => "Revoked",
            Self::Expired => "Expired",
            Self::NeedsUnlock => "NeedsUnlock",
            Self::Capacity => "Capacity",
            Self::OutcomeUnknown => "OutcomeUnknown",
            Self::Execution(_) => "Execution",
            Self::Payload(_) => "Payload",
        })
    }
}

impl ToolExecutionError {
    /// Typed native login failures retain their source; host admission can still
    /// classify explicit authentication without inspecting backend text.
    pub fn is_needs_unlock(&self) -> bool {
        match self {
            Self::NeedsUnlock => true,
            Self::Execution(cause) => {
                matches!(cause.downcast_ref::<nocterm_session::SessionError>(),Some(nocterm_session::SessionError::Connection(error)) if error.kind()==nocterm_session::ConnectionFailureKind::NeedsUnlock)
            }
            _ => false,
        }
    }
}
