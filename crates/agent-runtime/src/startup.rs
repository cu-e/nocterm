//! Workspace filesystem work belongs to the background startup task.
use nocterm_ai::{AgentError, sandbox::SandboxPolicy};
use std::path::{Path, PathBuf};
pub(super) fn workspace(
    candidate: &Path,
    private: bool,
    isolated: bool,
    private_dirs: &[PathBuf],
    shared_dirs: &[PathBuf],
) -> Result<(PathBuf, Option<SandboxPolicy>), AgentError> {
    if private {
        nocterm_core::paths::ensure_private_dir(candidate)
            .map_err(|error| AgentError::Io(error.to_string()))?;
    } else if !candidate.is_absolute() || !candidate.is_dir() {
        return Err(AgentError::Io(
            "Working directory must be an existing absolute directory.".into(),
        ));
    }
    let root =
        std::fs::canonicalize(candidate).map_err(|error| AgentError::Io(error.to_string()))?;
    let policy = isolated.then(|| {
        SandboxPolicy::new(
            &root,
            std::env::home_dir().as_deref(),
            private_dirs,
            shared_dirs,
        )
    });
    if let Some(policy) = &policy {
        policy
            .validate_workdir(&root)
            .map_err(|error| AgentError::Io(error.to_string()))?;
    }
    Ok((root, policy))
}

use futures::{
    FutureExt as _,
    channel::oneshot,
    future::{BoxFuture, Shared},
};
use nocterm_ai::{AgentConnection, AgentConnector, ConnectRequest};
use std::sync::Arc;
#[derive(Clone, Debug, thiserror::Error)]
pub(super) enum CleanupError {
    #[error("agent startup cleanup acknowledgement was lost")]
    AcknowledgementDropped,
    #[error("agent process cleanup failed: {0}")]
    Process(#[source] Arc<AgentError>),
}
pub(super) type Completion = Shared<BoxFuture<'static, Result<(), CleanupError>>>;
pub(super) struct StartupConfig {
    request: ConnectRequest,
    private: bool,
    isolated: bool,
    private_dirs: Vec<PathBuf>,
    shared_dirs: Vec<PathBuf>,
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
            private: workdir == services.workdir,
            request: ConnectRequest {
                launch,
                working_directory: workdir,
                terminal_auth: services.terminal_auth,
                sandbox: None,
                resources,
                cancellation,
            },
            isolated,
            private_dirs: services.private_dirs.clone(),
            shared_dirs: services.shared_dirs.clone(),
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
                Err(AgentError::CleanupUnconfirmed(message)) => Some(CleanupError::Process(
                    Arc::new(AgentError::CleanupUnconfirmed(message.clone())),
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
                    .map_err(|error| CleanupError::Process(Arc::new(error)))
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
    let (effective, sandbox) = workspace(
        &config.request.working_directory,
        config.private,
        config.isolated,
        &config.private_dirs,
        &config.shared_dirs,
    )?;
    config.request.working_directory = effective;
    config.request.sandbox = sandbox;
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
