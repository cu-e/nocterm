//! Per-attempt ownership survives an uncertain daemon-side create outcome.
use crate::{command::Commands, inspect::inspect};
use nocterm_containers::Engine;
use nocterm_project_model::{HostLocator, ProjectId, RootPath};
use nocterm_project_providers::{
    ContainerLocation, ContainerOrigin, CreationToken, OwnedContainer, ProjectSetupError,
    SetupPhase,
};

pub(crate) struct CreationAttempt {
    host: HostLocator,
    root: RootPath,
    engine: Engine,
    project: ProjectId,
    token: CreationToken,
}
impl CreationAttempt {
    pub(crate) fn new(
        host: HostLocator,
        root: RootPath,
        engine: Engine,
        project: ProjectId,
    ) -> Result<Self, ProjectSetupError> {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(ProjectSetupError::ownership_entropy)?;
        Ok(Self {
            host,
            root,
            engine,
            project,
            token: CreationToken::from_bytes(bytes),
        })
    }
    pub(crate) fn name(&self) -> String {
        format!("nocterm-{}-{}", self.project, self.token.as_str())
    }
    pub(crate) fn label(&self) -> String {
        format!("io.nocterm.attempt={}", self.token.as_str())
    }
    pub(crate) fn receipt(&self, id: &str) -> Result<OwnedContainer, ProjectSetupError> {
        let location = ContainerLocation::new(
            self.host.clone(),
            self.engine,
            id.to_owned(),
            self.root.clone(),
            ContainerOrigin::Created,
        )?;
        Ok(OwnedContainer::new(
            location,
            self.project,
            self.token.clone(),
        ))
    }
    /// Listing is only a candidate: deletion still requires exact full-ID inspection.
    pub(crate) async fn reconcile(
        &self,
        commands: &Commands,
        failure: ProjectSetupError,
    ) -> ProjectSetupError {
        let recovery = commands.recovery();
        match self.recover_owned(&recovery).await {
            Ok(Some(receipt)) => {
                let id = receipt.location().container();
                match recovery
                    .run(
                        SetupPhase::CreatingContainer,
                        self.engine.command().args(["rm", "--force", id]),
                    )
                    .await
                {
                    Ok(_) => failure,
                    Err(cleanup) => ProjectSetupError::CleanupFailed {
                        container: id.to_owned(),
                        failure: Box::new(failure),
                        cleanup: Box::new(cleanup),
                    },
                }
            }
            result => ProjectSetupError::CreationUncertain {
                engine: self.engine,
                attempt: self.token.clone(),
                failure: Box::new(failure),
                reconciliation: result.err().map(Box::new),
            },
        }
    }
    async fn recover_owned(
        &self,
        recovery: &Commands,
    ) -> Result<Option<OwnedContainer>, ProjectSetupError> {
        let output = recovery
            .run(
                SetupPhase::Inspecting,
                self.engine.command().args([
                    "ps",
                    "--all",
                    "--no-trunc",
                    "--quiet",
                    "--filter",
                    &format!("label={}", self.label()),
                ]),
            )
            .await?;
        let mut ids = output.split_whitespace();
        let Some(id) = ids.next() else {
            return Ok(None);
        };
        if ids.next().is_some() {
            return Ok(None);
        }
        let receipt = self.receipt(id)?;
        let current = inspect(recovery, self.engine, id).await?;
        if !current.owned_by(self.project, &self.token) {
            return Ok(None);
        }
        Ok(Some(receipt))
    }
}
