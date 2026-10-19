//! Background startup schedules an injected native workspace capability.
use futures::{
    FutureExt as _,
    channel::oneshot,
    future::{BoxFuture, Shared},
};
use nocterm_ai::{
    AgentConnector,
    connection::{ManagedAgentConnection, ManagedStartup, ManagedWorkspaceHandle},
    history::source::{ManagedCommandError, SourceError},
    model::{failure::ManagedNativeFailure, json::deferred::NativeAllocationError},
    workspace::WorkdirResolver,
};
use std::sync::Arc;
#[derive(Clone, Debug, thiserror::Error)]
pub(super) enum CleanupError {
    #[error("agent startup cleanup acknowledgement was lost")]
    AcknowledgementDropped,
    #[error("agent process cleanup failed: {0}")]
    Process(#[source] ManagedNativeFailure),
    #[error("agent process cleanup could not be admitted: {0}")]
    Admission(#[from] NativeAllocationError),
}
#[derive(Debug, thiserror::Error)]
pub(super) enum StartupError {
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(transparent)]
    Admission(#[from] NativeAllocationError),
    #[error(transparent)]
    Command(#[from] ManagedCommandError),
    #[error("Agent startup cancelled.")]
    Cancelled,
}
impl StartupError {
    fn unconfirmed_cleanup(&self) -> Option<CleanupError> {
        match self {
            Self::Command(ManagedCommandError::CleanupUnconfirmed(failure)) => {
                Some(CleanupError::Process(failure.clone()))
            }
            _ => None,
        }
    }
}
#[cfg(test)]
mod tests;
/// Runtime-owned acknowledgement that startup cleanup finished; it carries no native payload.
pub(super) type Completion = Shared<BoxFuture<'static, Result<(), CleanupError>>>;
pub(super) struct ManagedStartupConfig {
    startup: ManagedStartup,
    resolver: Arc<dyn WorkdirResolver>,
    connector: Arc<dyn AgentConnector>,
}
impl ManagedStartupConfig {
    pub(super) fn new(
        startup: ManagedStartup,
        resolver: Arc<dyn WorkdirResolver>,
        connector: Arc<dyn AgentConnector>,
    ) -> Self {
        Self {
            startup,
            resolver,
            connector,
        }
    }
}
pub(super) struct Delivery {
    result: Result<ManagedAgentConnection, StartupError>,
    workspace: Option<ManagedWorkspaceHandle>,
    decision: oneshot::Sender<bool>,
}
impl Delivery {
    pub(super) fn into_parts(
        self,
    ) -> (
        Result<ManagedAgentConnection, StartupError>,
        Option<ManagedWorkspaceHandle>,
        oneshot::Sender<bool>,
    ) {
        (self.result, self.workspace, self.decision)
    }
}
/// Detached background ownership survives foreground cancellation/App teardown.
/// It awaits acceptance, and confirms cleanup of any successfully spawned orphan.
pub(super) fn start(
    config: ManagedStartupConfig,
    executor: &gpui::BackgroundExecutor,
) -> (oneshot::Receiver<Delivery>, Completion) {
    let (delivered, delivery) = oneshot::channel();
    let (ack, completion) = oneshot::channel();
    executor
        .spawn(async move {
            let lifetime = config.startup.lifetime();
            let mut workspace = None;
            let result = launch(config, &mut workspace).await;
            let uncertain = result
                .as_ref()
                .err()
                .and_then(StartupError::unconfirmed_cleanup);
            let orphan = result
                .as_ref()
                .ok()
                .map(|connection| connection.commands().clone());
            let (decision, accepted) = oneshot::channel();
            let sent = delivered
                .send(Delivery {
                    result,
                    workspace,
                    decision,
                })
                .is_ok();
            let accepted = sent && accepted.await.unwrap_or(false);
            let cleanup = if let Some(error) = uncertain {
                Err(error)
            } else if !accepted && let Some(commands) = orphan {
                shutdown_orphan(commands, lifetime).await
            } else {
                Ok(())
            };
            let _ = ack.send(cleanup);
        })
        .detach();
    (
        delivery,
        async move {
            completion
                .await
                .unwrap_or(Err(CleanupError::AcknowledgementDropped))
        }
        .boxed()
        .shared(),
    )
}
/// The detached startup task is the only executor of an unaccepted orphan's shutdown.
async fn shutdown_orphan(
    commands: nocterm_ai::connection::ManagedCommandsHandle,
    lifetime: nocterm_ai::history::source::SourceLifetime,
) -> Result<(), CleanupError> {
    let cleanup = nocterm_ai::connection::ManagedProcessCleanup::new(commands, lifetime)?;
    let mut waiter = cleanup.capture()?;
    if let Some(driver) = waiter.take_driver() {
        driver.await;
    }
    waiter.await.map_err(CleanupError::Process)
}
async fn launch(
    config: ManagedStartupConfig,
    effective: &mut Option<ManagedWorkspaceHandle>,
) -> Result<ManagedAgentConnection, StartupError> {
    let ManagedStartupConfig {
        startup,
        resolver,
        connector,
    } = config;
    if startup.cancellation().is_cancelled() {
        return Err(StartupError::Cancelled);
    }
    let prepared = resolver
        .prepare_managed(startup.handle(), startup.lifetime())?
        .await?;
    let request = startup.prepared(prepared)?;
    *effective = Some(request.workspace_handle());
    if request.cancellation().is_cancelled() {
        return Err(StartupError::Cancelled);
    }
    Ok(connector.connect_managed(request)?.await?)
}
