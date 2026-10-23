pub(crate) mod paths;
pub(crate) mod platform;
pub(crate) mod probe;
use crate::command::Commands;
use futures::FutureExt as _;
use nocterm_project_providers::{
    EnvironmentError, EnvironmentExecution, EnvironmentFuture, EnvironmentSnapshot,
    EnvironmentSource, LanguageSelection, SetupProgress, WorkspaceEnvironment,
};
use nocterm_session::{ExecRequest, HostExec, local_fs::CancelFlag};
use std::sync::Arc;

#[derive(Clone, Copy, Default)]
pub struct NativeWorkspaceEnvironment;
impl WorkspaceEnvironment for NativeWorkspaceEnvironment {
    fn inspect(
        &self,
        source: EnvironmentSource,
        cancel: CancelFlag,
    ) -> EnvironmentFuture<EnvironmentSnapshot> {
        async move {
            let commands = Commands::new(
                Arc::new(EnvironmentExec {
                    source: source.clone(),
                }),
                cancel,
                Arc::new(|_| {}),
            );
            probe::inspect(&commands, &source).await
        }
        .boxed()
    }
    fn install(
        &self,
        source: EnvironmentSource,
        selections: Vec<LanguageSelection>,
        cancel: CancelFlag,
        progress: SetupProgress,
    ) -> EnvironmentFuture<EnvironmentSnapshot> {
        async move {
            nocterm_project_providers::validate_language_selections(&selections)?;
            let commands = Commands::new(
                Arc::new(EnvironmentExec {
                    source: source.clone(),
                }),
                cancel,
                progress,
            );
            let snapshot = probe::inspect(&commands, &source).await?;
            install(&commands, &snapshot, &selections, false).await?;
            probe::inspect(&commands, &source).await
        }
        .boxed()
    }
}
pub(crate) async fn install(
    commands: &Commands,
    snapshot: &EnvironmentSnapshot,
    selections: &[LanguageSelection],
    freshly_created: bool,
) -> Result<(), EnvironmentError> {
    for selection in selections {
        snapshot.support(selection)?;
    }
    let platform = platform::InstallPlatform::new(snapshot, freshly_created)?;
    let paths = paths::InstallPaths::new(&snapshot.account.home)?;
    platform.prepare(commands, snapshot, selections).await?;
    let mut completed = Vec::new();
    for selection in selections {
        let request = crate::languages::installation(selection, &platform, &paths)?;
        if let Err(error) = commands
            .run(
                nocterm_project_providers::SetupPhase::InstallingTools,
                request,
            )
            .await
        {
            let error = paths::installation_error(error, selection);
            return Err(if completed.is_empty() {
                error
            } else {
                EnvironmentError::PartialInstall {
                    completed,
                    cause: Box::new(error),
                }
            });
        }
        completed.push(selection.language());
    }
    Ok(())
}
struct EnvironmentExec {
    source: EnvironmentSource,
}
impl HostExec for EnvironmentExec {
    fn exec(
        &self,
        request: ExecRequest,
    ) -> nocterm_session::ExecFuture<nocterm_session::ExecOutput> {
        match self.source.execution() {
            EnvironmentExecution::Host(exec) => {
                let request = if let Ok(location) =
                    nocterm_project_providers::ContainerLocation::from_target(self.source.target())
                {
                    crate::prepare::container_exec(&location, request, false)
                } else {
                    request
                };
                exec.exec(request)
            }
            EnvironmentExecution::Scoped(exec) => exec.exec(request),
        }
    }
}

pub(crate) async fn install_created(
    commands: &Commands,
    location: &nocterm_project_providers::ContainerLocation,
    selections: &[LanguageSelection],
) -> Result<(), EnvironmentError> {
    let source = EnvironmentSource::host(
        location.to_target().map_err(EnvironmentError::from)?,
        commands.executor(),
    )?;
    let scoped = commands.with_executor(Arc::new(EnvironmentExec {
        source: source.clone(),
    }));
    let snapshot = probe::inspect(&scoped, &source).await?;
    if selections.is_empty() {
        return platform::InstallPlatform::new(&snapshot, true)?
            .prepare(&scoped, &snapshot, selections)
            .await;
    }
    install(&scoped, &snapshot, selections, true).await
}
#[cfg(test)]
mod tests;
