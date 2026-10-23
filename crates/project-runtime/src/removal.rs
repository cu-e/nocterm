//! Explicit exact-ID container lifecycle actions; never creation rollback or pruning.
use crate::{command::Commands, inspect::inspect};
use futures::FutureExt as _;
use nocterm_project_providers::{
    ContainerAction, ContainerActionError, ContainerActionFuture, ContainerActionOutcome,
    ContainerActionSource, ProjectSetupError, SetupPhase, WorkspaceContainerActions,
};
use nocterm_session::local_fs::CancelFlag;
use std::sync::Arc;

pub struct NativeWorkspaceContainerActions;
impl WorkspaceContainerActions for NativeWorkspaceContainerActions {
    fn apply(
        &self,
        source: ContainerActionSource,
        action: ContainerAction,
        cancel: CancelFlag,
    ) -> ContainerActionFuture {
        async move {
            let commands = Commands::new(source.executor(), cancel, Arc::new(|_| {})).lifecycle();
            let location = source.location();
            let engine = location.engine();
            let id = location.container();
            let inspected = inspect(&commands, engine, id).await.map_err(action_error)?;
            let running = match inspected.state.status.as_str() {
                "running" | "restarting" | "paused" => true,
                "exited" | "created" | "dead" => false,
                _ => return Err(action_error(ProjectSetupError::UnsupportedState)),
            };
            if running {
                commands
                    .run(
                        SetupPhase::Inspecting,
                        engine.command().args(["stop", "--time", "10", id]),
                    )
                    .await
                    .map_err(action_error)?;
            }
            if action == ContainerAction::Delete {
                commands
                    .run(SetupPhase::Inspecting, engine.command().args(["rm", id]))
                    .await
                    .map_err(|error| {
                        let error = action_error(error);
                        if running {
                            ContainerActionError::StoppedBeforeFailure {
                                cause: Box::new(error),
                            }
                        } else {
                            error
                        }
                    })?;
            }
            Ok(ContainerActionOutcome { action })
        }
        .boxed()
    }
}
fn action_error(error: ProjectSetupError) -> ContainerActionError {
    match error {
        ProjectSetupError::Canceled => ContainerActionError::Canceled,
        ProjectSetupError::TimedOut => ContainerActionError::TimedOut,
        ProjectSetupError::ContainerMissing => ContainerActionError::IdentityMismatch,
        ProjectSetupError::CommandFailed {
            phase,
            status,
            mut stderr_tail,
        } => {
            if stderr_tail.len() > 4096 {
                let mut start = stderr_tail.len() - 4096;
                while !stderr_tail.is_char_boundary(start) {
                    start += 1;
                }
                stderr_tail = stderr_tail[start..].to_owned();
            }
            ContainerActionError::Runtime(ProjectSetupError::CommandFailed {
                phase,
                status,
                stderr_tail,
            })
        }
        error => ContainerActionError::Runtime(error),
    }
}
