//! Ordinary session file access and optional workspace starting-directory metadata.
use crate::SessionContext;
use nocterm_project_model::{ProjectId, RootPath};
use nocterm_session::{HostAddress, RemoteFs};
use std::sync::Arc;

#[derive(Clone)]
pub struct ExplorerSource {
    session: Option<SessionContext>,
    directory: Option<RootPath>,
    project: Option<ProjectId>,
    local: bool,
}
impl ExplorerSource {
    pub fn session(session: SessionContext) -> Self {
        Self {
            session: Some(session),
            directory: None,
            project: None,
            local: false,
        }
    }
    pub fn folder(project: ProjectId, root: RootPath, session: SessionContext) -> Self {
        Self {
            session: Some(session),
            directory: Some(root),
            project: Some(project),
            local: false,
        }
    }
    pub fn local_folder(project: ProjectId, root: RootPath) -> Self {
        Self {
            session: None,
            directory: Some(root),
            project: Some(project),
            local: true,
        }
    }
    pub fn is_local(&self) -> bool {
        self.local
    }
    pub fn unavailable_project(project: ProjectId) -> Self {
        Self {
            session: None,
            directory: None,
            project: Some(project),
            local: false,
        }
    }
    pub fn project_id(&self) -> Option<ProjectId> {
        self.project
    }
    pub fn address(&self) -> Option<HostAddress> {
        self.session.as_ref().map(SessionContext::address)
    }
    pub fn connected(&self) -> bool {
        self.local
            || self
                .session
                .as_ref()
                .is_some_and(|session| session.connected)
    }
    pub fn filesystem(&self) -> Option<Arc<dyn RemoteFs>> {
        self.session.as_ref().and_then(|session| session.fs.clone())
    }
    pub fn initial_directory(&self) -> Option<&str> {
        self.directory.as_ref().map(RootPath::as_str)
    }
    pub fn same_as(&self, other: &Self) -> bool {
        if self.is_local() != other.is_local()
            || self.project_id() != other.project_id()
            || self.initial_directory() != other.initial_directory()
            || self.address() != other.address()
            || self.connected() != other.connected()
        {
            return false;
        }
        match (self.filesystem(), other.filesystem()) {
            (Some(a), Some(b)) => Arc::ptr_eq(&a, &b),
            (None, None) => true,
            _ => false,
        }
    }
}
