use nocterm_core::SharedError;

#[derive(Clone, Debug, thiserror::Error)]
pub enum ProjectSetupError {
    #[error("the workspace setup input is invalid: {0}")]
    InvalidInput(&'static str),
    #[error("this workspace provider is unavailable")]
    ProviderUnavailable,
    #[error("the workspace resource version is unsupported")]
    UnsupportedVersion,
    #[error("the workspace resource reference is invalid")]
    InvalidReference,
    #[error("workspace preparation was canceled")]
    Canceled,
    #[error("workspace creation ownership could not be allocated")]
    OwnershipEntropy(#[source] nocterm_core::SharedCause),
    #[error(
        "could not confirm {engine:?} container cleanup; recover by creation label io.nocterm.attempt={attempt}"
    )]
    CreationUncertain {
        engine: nocterm_containers::Engine,
        attempt: crate::CreationToken,
        #[source]
        failure: Box<ProjectSetupError>,
        reconciliation: Option<Box<ProjectSetupError>>,
    },
    #[error("workspace language environment is unsupported or could not be prepared")]
    Environment(#[source] Box<EnvironmentError>),
    #[error("workspace preparation timed out")]
    TimedOut,
    #[error("the selected container cannot be used in its current state")]
    UnsupportedState,
    #[error("the Git destination must be an empty directory; existing files were retained")]
    CloneDestinationNotEmpty,
    #[error("the project directory cannot be accessed")]
    DirectoryUnavailable,
    #[error("Git is not installed in the selected workspace")]
    GitUnavailable,
    #[error("the selected container no longer exists or changed identity")]
    ContainerMissing,
    #[error("cleanup refused a container without the original ownership label")]
    CleanupRefused,
    #[error("{phase} failed (exit status {status:?})")]
    CommandFailed {
        phase: &'static str,
        status: Option<u32>,
        stderr_tail: String,
    },
    #[error(transparent)]
    Execution(#[from] nocterm_session::ExecError),
    #[error(transparent)]
    Containers(SharedError<nocterm_containers::ContainersError>),
    #[error(transparent)]
    Validation(#[from] nocterm_project_model::ProjectValidationError),
    #[error("the container returned invalid inspection data")]
    Codec(#[source] SharedError<serde_json::Error>),
    #[error("workspace preparation failed; container {container} could not be removed")]
    CleanupFailed {
        container: String,
        #[source]
        failure: Box<ProjectSetupError>,
        cleanup: Box<ProjectSetupError>,
    },
}
impl From<serde_json::Error> for ProjectSetupError {
    fn from(value: serde_json::Error) -> Self {
        Self::Codec(SharedError::from_error(value))
    }
}

impl From<nocterm_containers::ContainersError> for ProjectSetupError {
    fn from(value: nocterm_containers::ContainersError) -> Self {
        Self::Containers(SharedError::from_error(value))
    }
}

impl ProjectSetupError {
    pub fn ownership_entropy(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::OwnershipEntropy(nocterm_core::SharedCause::new(source))
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum EnvironmentError {
    #[error("invalid environment request: {0}")]
    InvalidInput(&'static str),
    #[error("the workspace environment is unavailable")]
    Unavailable,
    #[error("the original workspace connection is disconnected")]
    Disconnected,
    #[error("the original workspace environment changed")]
    StaleSource,
    #[error("another installation is running in this environment")]
    Busy,
    #[error("this operating system is unsupported: {0}")]
    UnsupportedOs(String),
    #[error("this CPU architecture is unsupported: {0}")]
    UnsupportedArchitecture(String),
    #[error("{language:?} version is unsupported: {reason}")]
    UnsupportedVersion {
        language: crate::ProgrammingLanguage,
        reason: &'static str,
    },
    #[error("{program} is required; install {packages} in this environment first")]
    MissingPrerequisite {
        program: &'static str,
        packages: &'static str,
    },
    #[error(
        "distribution package installation requires root; choose a managed numeric version instead"
    )]
    PrivilegeRequired,
    #[error("the user's managed tool directory is not writable or has unsafe links")]
    ReadOnlyHome,
    #[error("environment operation was canceled")]
    Canceled,
    #[error("environment operation timed out")]
    TimedOut,
    #[error("could not inspect the workspace environment")]
    Probe(#[source] Box<ProjectSetupError>),
    #[error("language installation failed")]
    Installation(#[source] Box<ProjectSetupError>),
    #[error("some languages were installed before the operation failed")]
    PartialInstall {
        completed: Vec<crate::ProgrammingLanguage>,
        #[source]
        cause: Box<EnvironmentError>,
    },
}
impl From<ProjectSetupError> for EnvironmentError {
    fn from(error: ProjectSetupError) -> Self {
        match error {
            ProjectSetupError::Canceled => Self::Canceled,
            ProjectSetupError::TimedOut => Self::TimedOut,
            error => Self::Installation(Box::new(error)),
        }
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum ContainerActionError {
    #[error("the original container server is unavailable")]
    Unavailable,
    #[error("the original container source changed")]
    StaleSource,
    #[error("another removal is running for this container")]
    Busy,
    #[error("the container action was canceled; its outcome may be unknown")]
    Canceled,
    #[error("the container action timed out; its outcome may be unknown")]
    TimedOut,
    #[error("the saved container is missing or its identity changed")]
    IdentityMismatch,
    #[error("the container stopped, but could not be deleted")]
    StoppedBeforeFailure {
        #[source]
        cause: Box<ContainerActionError>,
    },
    #[error("the container action failed; check its state before retrying: {0}")]
    Runtime(#[from] ProjectSetupError),
}
