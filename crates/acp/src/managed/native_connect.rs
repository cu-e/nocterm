//! Actual managed startup binds paid protocol work to the original process owner.
use super::{budget::SourceBudget, diagnostics, native_process, redaction::DiagnosticRedactor};
use agent_client_protocol::managed::ManagedError;
use futures::future::{Either, select};
use nocterm_ai::{
    connection::{ManagedAgentConnection, ManagedConnectRequest},
    history::source::{ManagedCommandError, ManagedSourceFuture, SourceErrorSlot},
    model::json::deferred::{JsonDecodeError, NativeAllocationError},
};
use std::{io, path::PathBuf, sync::Arc};
#[derive(Debug, thiserror::Error)]
pub(crate) enum Cause {
    #[error(transparent)]
    Spawn(#[from] native_process::Error),
    #[error(transparent)]
    Routing(#[from] ManagedError<JsonDecodeError>),
    #[error(transparent)]
    Decode(#[from] super::error::DecodeError<JsonDecodeError>),
    #[error(transparent)]
    Native(#[from] JsonDecodeError),
    #[error(transparent)]
    Admission(#[from] NativeAllocationError),
    #[error(transparent)]
    Diagnostic(#[from] diagnostics::Error),
    #[error(transparent)]
    Cleanup(#[from] crate::process::CleanupError),
    #[error(transparent)]
    Driver(#[from] super::driver::DriverError),
    #[error("Cannot start agent driver: {0}")]
    Worker(#[source] io::Error),
    #[error("Agent {0} unavailable")]
    Pipe(&'static str),
    #[error("Agent startup cancelled")]
    Cancelled,
    #[error("Agent exited during initialization")]
    Exited,
    #[error("managed diagnostic rules were not prepared before startup")]
    RulesUnavailable,
}
pub(super) fn failure(error: Cause, ticket: SourceErrorSlot<Cause>) -> ManagedCommandError {
    let admission = match &error {
        Cause::Admission(error) => Some(*error),
        Cause::Native(error) => super::native_admission::json(error),
        Cause::Routing(error) => super::native_admission::sdk(error),
        Cause::Spawn(error) => super::native_admission::process(error),
        Cause::Driver(error) => error.admission(),
        Cause::Decode(error) => super::native_admission::decode(error),
        Cause::Diagnostic(error) => super::native_admission::diagnostic(error),
        _ => None,
    };
    let cleanup = matches!(error, Cause::Cleanup(_));
    match admission {
        Some(error) => ManagedCommandError::Admission(error),
        None if cleanup => ManagedCommandError::CleanupUnconfirmed(ticket.capture(error)),
        None => ManagedCommandError::Native(ticket.capture(error)),
    }
}
pub(crate) struct Preparation {
    helper: Option<Arc<PathBuf>>,
    rules: Option<Arc<DiagnosticRedactor>>,
}
impl Preparation {
    pub(crate) fn new(
        helper: Option<Arc<PathBuf>>,
        rules: Option<Arc<DiagnosticRedactor>>,
    ) -> Self {
        Self { helper, rules }
    }
    pub(crate) fn operation(
        self,
        request: ManagedConnectRequest,
    ) -> Result<
        ManagedSourceFuture<Result<ManagedAgentConnection, ManagedCommandError>>,
        NativeAllocationError,
    > {
        let lifetime = request.lifetime();
        let budget = SourceBudget::new(request.context().clone());
        let ticket = budget.context().prepare_error::<Cause>(0)?;
        ManagedSourceFuture::new(lifetime, async move {
            match self.run(request, budget).await {
                Ok(connection) => Ok(connection),
                Err(Failure::Managed(error)) => Err(error),
                Err(Failure::Native(error)) => Err(failure(error, ticket)),
            }
        })
    }
    async fn run(
        self,
        request: ManagedConnectRequest,
        budget: SourceBudget,
    ) -> Result<ManagedAgentConnection, Failure> {
        let rules = self.rules.ok_or(Cause::RulesUnavailable)?;
        let (startup, workspace, cancellation, lifetime) = request.into_native_parts();
        if cancellation.is_cancelled() {
            return Err(Cause::Cancelled.into());
        }
        let cancelled = cancellation.cancelled_managed().map_err(Cause::from)?;
        let spawned = native_process::Preparation::new(budget.clone())
            .spawn(
                &startup,
                &workspace,
                self.helper.as_deref().map(PathBuf::as_path),
                &rules,
            )
            .map_err(Cause::from)?;
        let group = spawned.group().clone();
        let mut guard = Connecting(Some(group.clone()));
        let launched =
            super::native_launch::Launch::new(budget, rules).start(spawned, startup, lifetime);
        let mut ready_receiver = match launched {
            Ok(receiver) => receiver,
            Err(error) => {
                group.shutdown_native().await.map_err(Cause::Cleanup)?;
                return Err(error.into());
            }
        };
        let ready = futures::future::poll_fn(|cx| ready_receiver.poll_recv(cx));
        futures::pin_mut!(ready, cancelled);
        let result = match select(ready, cancelled).await {
            Either::Left((Some(result), _)) => result.map_err(Failure::Managed),
            Either::Left((None, _)) => Err(Cause::Exited.into()),
            Either::Right(_) => Err(Cause::Cancelled.into()),
        };
        if result.is_ok() {
            guard.0.take();
        } else {
            group.shutdown_native().await.map_err(Cause::Cleanup)?;
        }
        result
    }
}
struct Connecting(Option<Arc<crate::process::ProcessGroup>>);
impl Drop for Connecting {
    fn drop(&mut self) {
        if let Some(group) = &self.0 {
            group.kill();
        }
    }
}
enum Failure {
    Managed(ManagedCommandError),
    Native(Cause),
}
impl From<Cause> for Failure {
    fn from(error: Cause) -> Self {
        Self::Native(error)
    }
}

#[cfg(all(test, unix))]
mod tests;
