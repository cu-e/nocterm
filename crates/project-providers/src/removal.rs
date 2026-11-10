//! Explicit user-confirmed actions, separate from creation rollback authority.
use crate::{ContainerActionError, ContainerLocation};
use futures::future::BoxFuture;
use nocterm_project_model::HostLocator;
use nocterm_session::{HostExec, local_fs::CancelFlag};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerAction {
    Stop,
    Delete,
}

/// A complete saved location paired with its original unscoped host capability.
#[derive(Clone)]
pub struct ContainerActionSource {
    location: ContainerLocation,
    executor: Arc<dyn HostExec>,
}
impl ContainerActionSource {
    pub fn new(
        location: ContainerLocation,
        captured_host: &HostLocator,
        executor: Arc<dyn HostExec>,
    ) -> Result<Self, ContainerActionError> {
        if location.host() != captured_host {
            return Err(ContainerActionError::StaleSource);
        }
        captured_host
            .validate()
            .map_err(crate::ProjectSetupError::from)?;
        Ok(Self { location, executor })
    }
    pub fn location(&self) -> &ContainerLocation {
        &self.location
    }
    pub fn executor(&self) -> Arc<dyn HostExec> {
        self.executor.clone()
    }
    /// Profile aliases and project directories do not distinguish engine resources.
    pub fn key(&self) -> ContainerActionKey {
        let host = match self.location.host() {
            HostLocator::Ssh { target, via, .. } => HostLocator::SshDirect {
                target: target.clone(),
                via: via.clone(),
            },
            host => host.clone(),
        };
        ContainerActionKey {
            host,
            engine: self.location.engine(),
            container: self.location.container().to_owned(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerActionKey {
    host: HostLocator,
    engine: nocterm_containers::Engine,
    container: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContainerActionOutcome {
    pub action: ContainerAction,
}
pub type ContainerActionFuture =
    BoxFuture<'static, Result<ContainerActionOutcome, ContainerActionError>>;
pub trait WorkspaceContainerActions: Send + Sync + 'static {
    fn apply(
        &self,
        source: ContainerActionSource,
        action: ContainerAction,
        cancel: CancelFlag,
    ) -> ContainerActionFuture;
}
