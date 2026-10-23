use crate::{ContainerLocation, LanguageSelection, ProgrammingLanguage, ProjectSetupError};
use futures::future::BoxFuture;
use nocterm_containers::{Container, ContainersError, Engine, Image};
use nocterm_project_model::{HostLocator, ProjectId, ProjectTarget, RootPath};
use nocterm_session::{HostExec, Transport, local_fs::CancelFlag};
use std::sync::Arc;

pub type SetupFuture<T> = BoxFuture<'static, Result<T, ProjectSetupError>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceImage {
    Ubuntu,
    Debian,
    Custom(String),
}
impl WorkspaceImage {
    pub fn reference(&self) -> &str {
        match self {
            Self::Ubuntu => "docker.io/library/ubuntu:24.04",
            Self::Debian => "docker.io/library/debian:bookworm",
            Self::Custom(value) => value,
        }
    }
    pub fn validate(&self) -> Result<(), ProjectSetupError> {
        let value = self.reference();
        if value.is_empty()
            || value.len() > 512
            || value.starts_with('-')
            || value.chars().any(char::is_whitespace)
            || value.chars().any(char::is_control)
        {
            return Err(ProjectSetupError::InvalidInput(
                "choose a valid image reference",
            ));
        }
        Ok(())
    }
}

/// Git sources are creation inputs, never persisted credentials or shell fragments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitRepository(String);
impl GitRepository {
    pub fn parse(value: String) -> Result<Self, ProjectSetupError> {
        let valid_scheme = value.starts_with("https://")
            || value.starts_with("ssh://")
            || value.starts_with("git@");
        let https_authority = value
            .strip_prefix("https://")
            .and_then(|tail| tail.split('/').next());
        if !valid_scheme
            || value.len() > 2048
            || value.chars().any(char::is_control)
            || value.chars().any(char::is_whitespace)
            || value
                .strip_prefix("ssh://")
                .and_then(|tail| tail.split('/').next())
                .is_some_and(|authority| {
                    authority
                        .split('@')
                        .next()
                        .is_some_and(|user| user.contains(':'))
                })
            || value.contains(['?', '#'])
            || https_authority.is_some_and(|authority| authority.contains('@'))
        {
            return Err(ProjectSetupError::InvalidInput(
                "use a nonsecret HTTPS or SSH Git repository",
            ));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub enum SetupMode {
    Direct {
        root: RootPath,
    },
    Attach {
        engine: Engine,
        container: String,
        root: Option<RootPath>,
    },
    Create {
        engine: Engine,
        image: WorkspaceImage,
        languages: Vec<LanguageSelection>,
        root: Option<RootPath>,
    },
}
#[derive(Clone, Debug)]
pub struct ProjectSetupRequest {
    pub project: ProjectId,
    pub host: HostLocator,
    pub mode: SetupMode,
    pub repository: Option<GitRepository>,
}
impl ProjectSetupRequest {
    pub fn validate(&self) -> Result<(), ProjectSetupError> {
        self.host.validate()?;
        match &self.mode {
            SetupMode::Create {
                image, languages, ..
            } => {
                image.validate()?;
                let mut unique = std::collections::BTreeSet::new();
                if languages
                    .iter()
                    .any(|selection| !unique.insert(selection.language()))
                {
                    return Err(ProjectSetupError::InvalidInput(
                        "choose each language only once",
                    ));
                }
                if languages.len() > ProgrammingLanguage::ALL.len() {
                    return Err(ProjectSetupError::InvalidInput(
                        "too many language selections",
                    ));
                }
            }
            SetupMode::Attach { container, .. }
                if container.len() != 64 || !container.bytes().all(|b| b.is_ascii_hexdigit()) =>
            {
                return Err(ProjectSetupError::InvalidReference);
            }
            _ => {}
        }
        Ok(())
    }
}

/// Fresh nonsecret per-attempt ownership, supplied by the native preparation adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreationToken(String);
impl CreationToken {
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Display for CreationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug)]
pub struct OwnedContainer {
    location: ContainerLocation,
    project: ProjectId,
    attempt: CreationToken,
}
impl OwnedContainer {
    pub fn new(location: ContainerLocation, project: ProjectId, attempt: CreationToken) -> Self {
        Self {
            location,
            project,
            attempt,
        }
    }
    pub fn attempt(&self) -> &CreationToken {
        &self.attempt
    }
    pub fn location(&self) -> &ContainerLocation {
        &self.location
    }
    pub fn project(&self) -> ProjectId {
        self.project
    }
}
#[derive(Clone, Debug)]
pub struct PreparedProject {
    pub target: ProjectTarget,
    pub owned: Option<OwnedContainer>,
}
#[derive(Debug)]
pub struct EngineInventory {
    pub engine: Engine,
    pub containers: Result<Vec<Container>, ContainersError>,
    pub images: Result<Vec<Image>, ContainersError>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupPhase {
    Inspecting,
    PullingImage,
    CreatingContainer,
    StartingContainer,
    InstallingTools,
    PreparingDirectory,
    CloningRepository,
}
impl SetupPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Inspecting => "Checking workspace",
            Self::PullingImage => "Pulling image",
            Self::CreatingContainer => "Creating container",
            Self::StartingContainer => "Starting container",
            Self::InstallingTools => "Installing languages",
            Self::PreparingDirectory => "Preparing directory",
            Self::CloningRepository => "Cloning repository",
        }
    }
}
pub type SetupProgress = Arc<dyn Fn(crate::SetupUpdate) + Send + Sync>;
pub trait ProjectPreparation: Send + Sync + 'static {
    fn discover(&self, exec: Arc<dyn HostExec>) -> SetupFuture<Vec<EngineInventory>>;
    fn prepare(
        &self,
        exec: Arc<dyn HostExec>,
        request: ProjectSetupRequest,
        cancel: CancelFlag,
        progress: SetupProgress,
    ) -> SetupFuture<PreparedProject>;
    fn discard_owned(&self, exec: Arc<dyn HostExec>, receipt: OwnedContainer) -> SetupFuture<()>;
}
pub type LocalTransportFactory =
    Arc<dyn Fn(nocterm_session::ShellLaunch) -> Arc<dyn Transport> + Send + Sync>;
pub trait ContainerSessionFactory: Send + Sync + 'static {
    fn wrap(&self, base: Arc<dyn Transport>, location: ContainerLocation) -> Arc<dyn Transport>;
    fn wrap_local(
        &self,
        factory: LocalTransportFactory,
        location: ContainerLocation,
    ) -> Arc<dyn Transport>;
}
