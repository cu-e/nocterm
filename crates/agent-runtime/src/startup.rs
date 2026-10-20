//! Background startup schedules an injected native workspace capability.
use futures::{
    FutureExt as _,
    channel::oneshot,
    future::{BoxFuture, Shared},
};
use nocterm_ai::{AgentConnection, AgentConnector, ConnectRequest};
use nocterm_ai::{
    AgentError,
    workspace::{IsolationPaths, WorkdirRequest, WorkdirResolver},
};
use std::path::PathBuf;
use std::sync::Arc;
#[derive(Clone, Debug, thiserror::Error)]
pub(super) enum CleanupError {
    #[error("agent startup cleanup acknowledgement was lost")]
    AcknowledgementDropped,
    #[error("agent process cleanup failed: {0}")]
    Process(#[source] nocterm_core::SharedError<AgentError>),
}
impl CleanupError {
    pub(super) fn process(error: AgentError) -> Self {
        Self::Process(nocterm_core::SharedError::from_error(error))
    }
}
#[cfg(test)]
mod tests;
pub(super) type Completion = Shared<BoxFuture<'static, Result<(), CleanupError>>>;
pub(super) struct StartupConfig {
    request: ConnectRequest,
    preparation: WorkdirRequest,
    resolver: Arc<dyn WorkdirResolver>,
}
impl StartupConfig {
    pub(super) fn for_connection(
        services: &super::RuntimeServices,
        launch: nocterm_ai::AgentLaunch,
        workdir: PathBuf,
        isolated: bool,
        resources: nocterm_ai::AgentResourceSettings,
        cancellation: nocterm_ai::ConnectionCancellation,
    ) -> Self {
        Self {
            preparation: WorkdirRequest {
                candidate: workdir.clone(),
                create_private: workdir == services.workdir,
                isolation: isolated.then(|| IsolationPaths {
                    private_dirs: services.private_dirs.clone(),
                    shared_dirs: services.shared_dirs.clone(),
                    home_state: services
                        .agents
                        .catalog()
                        .home_state()
                        .map(PathBuf::from)
                        .collect(),
                }),
            },
            resolver: services.workdir_resolver.clone(),
            request: ConnectRequest {
                launch,
                working_directory: workdir,
                terminal_auth: services.terminal_auth,
                sandbox: None,
                resources,
                cancellation,
            },
        }
    }
}
pub(super) struct Delivery {
    result: Result<AgentConnection, AgentError>,
    effective: PathBuf,
    decision: oneshot::Sender<bool>,
}
impl Delivery {
    pub(super) fn into_parts(
        self,
    ) -> (
        Result<AgentConnection, AgentError>,
        PathBuf,
        oneshot::Sender<bool>,
    ) {
        (self.result, self.effective, self.decision)
    }
}
/// Detached background ownership survives foreground cancellation/App teardown.
/// It awaits acceptance, and confirms cleanup of any successfully spawned orphan.
pub(super) fn start(
    mut config: StartupConfig,
    connector: Arc<dyn AgentConnector>,
    executor: &gpui::BackgroundExecutor,
) -> (oneshot::Receiver<Delivery>, Completion) {
    let (delivered, delivery) = oneshot::channel();
    let (ack, completion) = oneshot::channel();
    executor
        .spawn(async move {
            let result = launch(&mut config, connector).await;
            let uncertain = match &result {
                Err(AgentError::CleanupUnconfirmed(message)) => Some(CleanupError::process(
                    AgentError::CleanupUnconfirmed(message.clone()),
                )),
                _ => None,
            };
            let orphan = result
                .as_ref()
                .ok()
                .map(|connection| connection.commands.clone());
            let (decision, accepted) = oneshot::channel();
            let sent = delivered
                .send(Delivery {
                    result,
                    effective: config.request.working_directory,
                    decision,
                })
                .is_ok();
            let accepted = sent && accepted.await.unwrap_or(false);
            let cleanup = if let Some(error) = uncertain {
                Err(error)
            } else if !accepted && let Some(commands) = orphan {
                commands
                    .shutdown_gracefully()
                    .await
                    .map_err(CleanupError::process)
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
async fn launch(
    config: &mut StartupConfig,
    connector: Arc<dyn AgentConnector>,
) -> Result<AgentConnection, AgentError> {
    if config.request.cancellation.is_cancelled() {
        return Err(AgentError::Io("Agent startup cancelled.".into()));
    }
    let prepared = config.resolver.prepare(config.preparation.clone()).await?;
    config.request.working_directory = prepared.root;
    config.request.sandbox = prepared.sandbox;
    if config.request.cancellation.is_cancelled() {
        return Err(AgentError::Io("Agent startup cancelled.".into()));
    }
    connector
        .connect(ConnectRequest {
            launch: config.request.launch.clone(),
            working_directory: config.request.working_directory.clone(),
            terminal_auth: config.request.terminal_auth,
            sandbox: config.request.sandbox.clone(),
            resources: config.request.resources.clone(),
            cancellation: config.request.cancellation.clone(),
        })
        .await
}
