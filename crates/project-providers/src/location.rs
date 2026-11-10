use nocterm_containers::Engine;
use nocterm_project_model::{
    HostLocator, ProjectTarget, ProviderId, ProviderResourceRef, RootPath,
};
use serde::{Deserialize, Serialize};

use crate::ProjectSetupError;

pub const CONTAINER_PROVIDER: &str = "nocterm.container";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerOrigin {
    Attached,
    Created,
}

/// Strict version-1 builtin payload; display names never supply execution identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ContainerRecord")]
pub struct ContainerLocation {
    host: HostLocator,
    engine: Engine,
    container: String,
    root: RootPath,
    origin: ContainerOrigin,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContainerRecord {
    host: HostLocator,
    engine: Engine,
    container: String,
    root: RootPath,
    origin: ContainerOrigin,
}
impl TryFrom<ContainerRecord> for ContainerLocation {
    type Error = ProjectSetupError;
    fn try_from(value: ContainerRecord) -> Result<Self, Self::Error> {
        Self::new(
            value.host,
            value.engine,
            value.container,
            value.root,
            value.origin,
        )
    }
}
impl ContainerLocation {
    /// Wraps a literal program in the original exact-container execution request.
    pub fn exec_request(
        &self,
        program: nocterm_session::ExecRequest,
        use_workspace_directory: bool,
    ) -> nocterm_session::ExecRequest {
        let mut request = self.engine().command().args(["exec", "--interactive"]);
        if use_workspace_directory {
            request = request.args(["--workdir", self.root().as_str()]);
        }
        request = request
            .arg(self.container())
            .arg(program.program)
            .args(program.args);
        request.stdin = program.stdin;
        request
    }

    pub fn new(
        host: HostLocator,
        engine: Engine,
        container: String,
        root: RootPath,
        origin: ContainerOrigin,
    ) -> Result<Self, ProjectSetupError> {
        host.validate()?;
        if container.len() != 64 || !container.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ProjectSetupError::InvalidReference);
        }
        Ok(Self {
            host,
            engine,
            container,
            root,
            origin,
        })
    }
    pub fn host(&self) -> &HostLocator {
        &self.host
    }
    pub fn engine(&self) -> Engine {
        self.engine
    }
    pub fn container(&self) -> &str {
        &self.container
    }
    pub fn root(&self) -> &RootPath {
        &self.root
    }
    pub fn origin(&self) -> &ContainerOrigin {
        &self.origin
    }
    pub fn with_root(&self, root: RootPath) -> Self {
        Self {
            root,
            ..self.clone()
        }
    }
    pub fn to_target(&self) -> Result<ProjectTarget, ProjectSetupError> {
        Ok(ProjectTarget::Provider {
            provider: ProviderId::parse(CONTAINER_PROVIDER)?,
            resource_version: 1,
            reference: ProviderResourceRef::new(serde_json::to_value(self)?)?,
        })
    }
    pub fn from_target(target: &ProjectTarget) -> Result<Self, ProjectSetupError> {
        let ProjectTarget::Provider {
            provider,
            resource_version,
            reference,
        } = target
        else {
            return Err(ProjectSetupError::InvalidReference);
        };
        if provider.as_str() != CONTAINER_PROVIDER {
            return Err(ProjectSetupError::ProviderUnavailable);
        }
        if *resource_version != 1 {
            return Err(ProjectSetupError::UnsupportedVersion);
        }
        let decoded: Self = serde_json::from_value(reference.value().clone())?;
        Self::new(
            decoded.host,
            decoded.engine,
            decoded.container,
            decoded.root,
            decoded.origin,
        )
    }
}
