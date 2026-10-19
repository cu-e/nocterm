//! Paid async preparation retains the original startup facts through cancellation.
use super::{Home, home::HomeFacts, paths::Paths};
use crate::managed::SourceBudget;
use agent_client_protocol::managed::ManagedError;
use nocterm_ai::{
    connection::ManagedStartupHandle,
    history::source::{ManagedCommandError, ManagedSourceFuture, SourceLifetime},
    model::{
        json::deferred::{JsonDecodeError, NativeAllocationError, NativeScratch},
        native::{NativeOwned, NativePathError},
    },
    workspace::{PreparedWorkspace, WorkdirError},
};
use nocterm_core::fs::{AdmittedFileError, PathAdmissionError};
#[derive(Debug, thiserror::Error)]
pub(crate) enum Cause {
    #[error("Could not resolve agent workspace path {path}: {source}",path=.0.path().display(),source=.0.native_source())]
    Resolve(#[source] AdmittedFileError<NativeScratch>),
    #[error("Could not create agent workspace at {path}: {source}",path=.0.path().display(),source=.0.native_source())]
    Create(#[source] AdmittedFileError<NativeScratch>),
    #[error(transparent)]
    Workspace(#[from] WorkdirError),
    #[error(transparent)]
    Json(JsonDecodeError),
    #[error(transparent)]
    Plan(#[from] PathAdmissionError),
    #[error(transparent)]
    Sdk(ManagedError<JsonDecodeError>),
    #[error(transparent)]
    Admission(#[from] NativeAllocationError),
    #[error("native path bytes changed after metadata validation")]
    Input,
    #[cfg(not(unix))]
    #[error("managed native workspace preparation is unavailable on this platform")]
    Platform,
}
impl From<JsonDecodeError> for Cause {
    fn from(error: JsonDecodeError) -> Self {
        match error {
            JsonDecodeError::Arithmetic => Self::Admission(NativeAllocationError::Arithmetic),
            JsonDecodeError::Limit {
                requested,
                available,
            } => Self::Admission(NativeAllocationError::Limit {
                requested,
                available,
            }),
            JsonDecodeError::Retention(error) => {
                Self::Admission(NativeAllocationError::Retention(error))
            }
            other => Self::Json(other),
        }
    }
}
impl From<NativePathError> for Cause {
    fn from(error: NativePathError) -> Self {
        match error {
            NativePathError::Admission(error) => error.into(),
            NativePathError::Plan(error) => Self::Plan(error),
        }
    }
}
impl From<ManagedError<JsonDecodeError>> for Cause {
    fn from(error: ManagedError<JsonDecodeError>) -> Self {
        match error {
            ManagedError::Refused(error) => error.into(),
            other => Self::Sdk(other),
        }
    }
}
pub(super) struct Preparation {
    budget: SourceBudget,
}
impl Preparation {
    #[expect(
        clippy::result_large_err,
        reason = "failures carry ledger-paid diagnostics by value; a Box would be an unpaid allocation"
    )]
    pub(super) fn operation(
        home: &Home,
        startup: ManagedStartupHandle,
        lifetime: SourceLifetime,
    ) -> Result<
        ManagedSourceFuture<Result<NativeOwned<PreparedWorkspace>, ManagedCommandError>>,
        NativeAllocationError,
    > {
        let budget = SourceBudget::new(lifetime.context_ref().clone());
        let ticket = budget.context().prepare_error::<Cause>(0)?;
        let home = HomeFacts::prepare(home, &budget);
        let this = Self { budget };
        ManagedSourceFuture::new(lifetime, async move {
            let result = home.and_then(|home| this.run(startup, home));
            match result {
                Ok(workspace) => Ok(workspace),
                Err(Cause::Admission(error)) => Err(ManagedCommandError::Admission(error)),
                Err(error) => Err(ManagedCommandError::Native(ticket.capture(error))),
            }
        })
    }
    #[expect(
        clippy::result_large_err,
        reason = "failures carry ledger-paid diagnostics by value; a Box would be an unpaid allocation"
    )]
    fn run(
        &self,
        startup: ManagedStartupHandle,
        home: HomeFacts,
    ) -> Result<NativeOwned<PreparedWorkspace>, Cause> {
        if !startup.belongs_to_source(self.budget.context()) {
            return Err(JsonDecodeError::ForeignLedger.into());
        }
        let startup = startup.facts();
        if !startup.working_directory().is_absolute() {
            return Err(WorkdirError::InvalidDirectory.into());
        }
        let paths = Paths::new(self.budget.clone());
        if startup.create_private() {
            paths.private(startup.working_directory())?;
        }
        let root = nocterm_ai::model::native::workspace::into_resolved_path(
            paths.required(startup.working_directory())?,
        )?;
        let policy = match startup.isolation() {
            None => NativeOwned::absent(self.budget.ledger())?,
            Some(isolation) => {
                let home = home.resolve(&self.budget)?;
                let policy = nocterm_ai::model::native::workspace::sandbox_policy(
                    self.budget.ledger(),
                    root.as_path(),
                    home.as_optional_path(),
                    isolation.private_dirs(),
                    isolation.shared_dirs(),
                )?;
                let resolved = paths.resolve(policy.get(), root.as_path())?;
                policy
                    .get()
                    .validate_workdir(resolved.get())
                    .map_err(WorkdirError::from)?;
                NativeOwned::supplied(policy)
            }
        };
        Ok(nocterm_ai::model::native::workspace::prepared_workspace(
            root, policy,
        )?)
    }
}
