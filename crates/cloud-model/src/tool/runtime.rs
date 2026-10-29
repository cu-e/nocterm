//! Host-only per-run resource views; a model never receives these collaborators.
use super::{ScopedToolExecutor, ToolAuthority, ToolExecutionError};
use crate::{
    authority::{RunBinding, SessionReceipt},
    identity::Fingerprint,
};
use futures::future::BoxFuture;
use nocterm_ai::TerminalCall;
use std::sync::Arc;

/// The public contract owns canonical cryptographic argument bytes. The adapter
/// maps the actual AI DTO; no production JSON/hash substitute is provided here.
pub trait ArgumentFingerprint: Send + Sync {
    fn fingerprint(&self, call: &TerminalCall) -> Result<Fingerprint, ToolExecutionError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunRetirement {
    Completed,
    Stopped,
    Revoked,
    Interrupted,
}

pub trait RunToolRuntime: ScopedToolExecutor {
    fn run(&self) -> &RunBinding;
    fn receipt(&self) -> &SessionReceipt;
    fn check_current(&self) -> Result<(), ToolExecutionError>;
    /// Immediately rejects new admission before cleanup or durable-control waits.
    fn fence(&self);
    /// Completion releases this foreground view, retaining independent original
    /// background jobs. Stop/revoke cleanup cannot promise remote reversal.
    fn retire(&self, reason: RunRetirement) -> BoxFuture<'static, Result<(), ToolExecutionError>>;
}

pub trait RunToolRuntimeFactory: Send + Sync {
    /// Attaches an already authenticated receipt or returns NeedsUnlock. No
    /// automatic login, secret discovery, cloned event reader or handle retarget.
    fn attach(
        &self,
        run: RunBinding,
        receipt: SessionReceipt,
        authority: Arc<dyn ToolAuthority>,
    ) -> BoxFuture<'static, Result<Arc<dyn RunToolRuntime>, ToolExecutionError>>;
}
