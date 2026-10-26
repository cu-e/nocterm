pub(crate) mod paths;
pub(crate) mod platform;
pub(crate) mod probe;
use futures::FutureExt as _;
use nocterm_project_providers::{
    CreatedWorkspaceEnvironment, EnvironmentCommandFactory, EnvironmentCommands, OwnedContainer,
};
use nocterm_project_providers::{
    EnvironmentError, EnvironmentExecution, EnvironmentFuture, EnvironmentSnapshot,
    EnvironmentSource, LanguageSelection, SetupProgress, WorkspaceEnvironment,
};
use nocterm_session::{ExecRequest, HostExec, local_fs::CancelFlag};
use std::sync::Arc;

#[derive(Clone)]
pub struct NativeWorkspaceEnvironment {
    commands: Arc<dyn EnvironmentCommandFactory>,
}
impl NativeWorkspaceEnvironment {
    pub fn new(commands: Arc<dyn EnvironmentCommandFactory>) -> Self {
        Self { commands }
    }
}
impl WorkspaceEnvironment for NativeWorkspaceEnvironment {
    fn inspect(
        &self,
        source: EnvironmentSource,
        cancel: CancelFlag,
    ) -> EnvironmentFuture<EnvironmentSnapshot> {
        let factory = self.commands.clone();
        async move {
            let commands = factory.create(
                Arc::new(EnvironmentExec {
                    source: source.clone(),
                }),
                cancel,
                Arc::new(|_| {}),
            );
            probe::inspect(commands.as_ref(), &source).await
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
        let factory = self.commands.clone();
        async move {
            nocterm_project_providers::validate_language_selections(&selections)?;
            let commands = factory.create(
                Arc::new(EnvironmentExec {
                    source: source.clone(),
                }),
                cancel,
                progress,
            );
            let snapshot = probe::inspect(commands.as_ref(), &source).await?;
            install(commands.as_ref(), &snapshot, &selections, false).await?;
            probe::inspect(commands.as_ref(), &source).await
        }
        .boxed()
    }
}
pub(crate) async fn install(
    commands: &dyn EnvironmentCommands,
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
                    location.exec_request(request, false)
                } else {
                    request
                };
                exec.exec(request)
            }
            EnvironmentExecution::Scoped(exec) => exec.exec(request),
        }
    }
}

impl CreatedWorkspaceEnvironment for NativeWorkspaceEnvironment {
    fn install_created(
        &self,
        commands: Arc<dyn EnvironmentCommands>,
        owned: OwnedContainer,
        selections: Vec<LanguageSelection>,
    ) -> EnvironmentFuture<()> {
        async move { install_created(commands.as_ref(), owned.location(), &selections).await }
            .boxed()
    }
}

pub(crate) async fn install_created(
    commands: &dyn EnvironmentCommands,
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
    let snapshot = probe::inspect(scoped.as_ref(), &source).await?;
    if selections.is_empty() {
        return platform::InstallPlatform::new(&snapshot, true)?
            .prepare(scoped.as_ref(), &snapshot, selections)
            .await;
    }
    install(scoped.as_ref(), &snapshot, selections, true).await
}
