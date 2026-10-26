//! Captured environment authority and ephemeral observations; no acquisition or installation I/O.
use crate::{ContainerLocation, LanguageSelection, ProgrammingLanguage, SetupProgress};
use futures::future::BoxFuture;
use nocterm_containers::Engine;
use nocterm_project_model::{HostLocator, ProjectTarget};
use nocterm_session::{HostExec, local_fs::CancelFlag};
use std::sync::Arc;

pub const MANAGED_ENVIRONMENT_RELATIVE: &str = ".local/share/nocterm/toolchains/env.sh";
pub type EnvironmentFuture<T> = BoxFuture<'static, Result<T, crate::EnvironmentError>>;
#[derive(Clone)]
pub enum EnvironmentExecution {
    Host(Arc<dyn HostExec>),
    Scoped(Arc<dyn HostExec>),
}
#[derive(Clone)]
pub struct EnvironmentSource {
    target: ProjectTarget,
    execution: EnvironmentExecution,
    host_metadata: Option<Arc<dyn HostExec>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvironmentKey {
    host: Option<HostLocator>,
    container: Option<(Engine, String)>,
}
impl EnvironmentSource {
    pub fn host(
        target: ProjectTarget,
        exec: Arc<dyn HostExec>,
    ) -> Result<Self, crate::EnvironmentError> {
        validate_target(&target)?;
        Ok(Self {
            target,
            execution: EnvironmentExecution::Host(exec.clone()),
            host_metadata: Some(exec),
        })
    }
    pub fn scoped(
        target: ProjectTarget,
        exec: Arc<dyn HostExec>,
        host_metadata: Option<Arc<dyn HostExec>>,
    ) -> Result<Self, crate::EnvironmentError> {
        validate_target(&target)?;
        Ok(Self {
            target,
            execution: EnvironmentExecution::Scoped(exec),
            host_metadata,
        })
    }
    pub fn target(&self) -> &ProjectTarget {
        &self.target
    }
    pub fn execution(&self) -> &EnvironmentExecution {
        &self.execution
    }
    pub fn host_metadata(&self) -> Option<Arc<dyn HostExec>> {
        self.host_metadata.clone()
    }
    pub fn key(&self) -> EnvironmentKey {
        match &self.target {
            ProjectTarget::HostFolder { host, .. } => EnvironmentKey {
                host: Some(environment_host(host)),
                container: None,
            },
            _ => {
                let location = ContainerLocation::from_target(&self.target).ok();
                EnvironmentKey {
                    host: location
                        .as_ref()
                        .map(|value| environment_host(value.host())),
                    container: location.map(|value| (value.engine(), value.container().to_owned())),
                }
            }
        }
    }
}

fn environment_host(host: &HostLocator) -> HostLocator {
    match host {
        HostLocator::Ssh { target, via, .. } => HostLocator::SshDirect {
            target: target.clone(),
            via: via.clone(),
        },
        host => host.clone(),
    }
}
fn validate_target(target: &ProjectTarget) -> Result<(), crate::EnvironmentError> {
    match target {
        ProjectTarget::HostFolder { host, .. } => host
            .validate()
            .map_err(|_| crate::EnvironmentError::InvalidInput("invalid environment host")),
        _ => ContainerLocation::from_target(target)
            .map(|_| ())
            .map_err(|_| crate::EnvironmentError::Unavailable),
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibcFamily {
    Glibc,
    Musl,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistributionFamily {
    Debian,
    Alpine,
    Arch,
    Unsupported,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvironmentOs {
    pub kernel: String,
    pub id: String,
    pub name: String,
    pub release: String,
    pub architecture: String,
    pub libc: LibcFamily,
    pub family: DistributionFamily,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvironmentAccount {
    pub user: String,
    pub uid: u32,
    pub home: String,
    pub writable_home: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolObservation {
    pub language: ProgrammingLanguage,
    pub version: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerFacts {
    pub engine: Engine,
    pub id: String,
    pub name: Option<String>,
    pub image: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvironmentSnapshot {
    pub os: EnvironmentOs,
    pub account: EnvironmentAccount,
    pub tools: Vec<ToolObservation>,
    pub container: Option<ContainerFacts>,
    pub available_programs: Vec<String>,
}
impl EnvironmentSnapshot {
    pub fn support(&self, selection: &LanguageSelection) -> Result<(), crate::EnvironmentError> {
        use crate::{EnvironmentError as E, VersionChoice};
        if self.os.kernel != "Linux" || self.os.family == DistributionFamily::Unsupported {
            return Err(E::UnsupportedOs(self.os.id.clone()));
        }
        if !matches!(self.os.architecture.as_str(), "x86_64" | "aarch64") {
            return Err(E::UnsupportedArchitecture(self.os.architecture.clone()));
        }
        if self.os.libc == LibcFamily::Unknown {
            return Err(E::MissingPrerequisite {
                program: "libc",
                packages: "a supported glibc or musl environment",
            });
        }
        if self.os.libc == LibcFamily::Musl
            && selection.language() == ProgrammingLanguage::Node
            && !matches!(selection.version(), VersionChoice::DistributionDefault)
        {
            return Err(E::UnsupportedVersion {
                language: selection.language(),
                reason: "official Node.js pinned archives require glibc; use distribution default on musl",
            });
        }
        if !self.account.writable_home {
            return Err(E::ReadOnlyHome);
        }
        Ok(())
    }
}
pub trait WorkspaceEnvironment: Send + Sync + 'static {
    fn inspect(
        &self,
        source: EnvironmentSource,
        cancel: CancelFlag,
    ) -> EnvironmentFuture<EnvironmentSnapshot>;
    fn install(
        &self,
        source: EnvironmentSource,
        selections: Vec<LanguageSelection>,
        cancel: CancelFlag,
        progress: SetupProgress,
    ) -> EnvironmentFuture<EnvironmentSnapshot>;
}
pub fn validate_language_selections(
    selections: &[LanguageSelection],
) -> Result<(), crate::EnvironmentError> {
    let mut seen = std::collections::BTreeSet::new();
    if selections.is_empty()
        || selections.len() > ProgrammingLanguage::ALL.len()
        || selections
            .iter()
            .any(|selection| !seen.insert(selection.language()))
    {
        return Err(crate::EnvironmentError::InvalidInput(
            "choose each requested language once",
        ));
    }
    Ok(())
}
