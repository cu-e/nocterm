//! Human creation retains an original connection or explicit local capability.
use std::path::PathBuf;

use gpui::{App, SharedString};
use nocterm_project_model::{HostLocator, ProjectValidationError};

use crate::ExpectedSession;

/// Inert parent identity for sidebar grouping and presentation after disconnect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectParentDescriptor {
    host: HostLocator,
    title: SharedString,
}

impl ProjectParentDescriptor {
    pub fn new(host: HostLocator, title: SharedString) -> Result<Self, ProjectParentError> {
        host.validate()?;
        Ok(Self { host, title })
    }
    pub fn host(&self) -> &HostLocator {
        &self.host
    }
    pub fn title(&self) -> &SharedString {
        &self.title
    }
}

/// Original creation target. Observation does not keep its shell alive.
#[derive(Clone)]
pub struct ProjectCreationSource {
    parent: ProjectParentDescriptor,
    expected: Option<ExpectedSession>,
    cwd: Option<PathBuf>,
    exec: Option<std::sync::Arc<dyn nocterm_session::HostExec>>,
    fs: Option<std::sync::Arc<dyn nocterm_session::RemoteFs>>,
}

impl ProjectCreationSource {
    pub fn new(
        parent: ProjectParentDescriptor,
        expected: ExpectedSession,
        cwd: Option<PathBuf>,
    ) -> Result<Self, ProjectParentError> {
        parent.host().validate()?;
        let source = Self {
            parent,
            expected: Some(expected),
            cwd,
            exec: None,
            fs: None,
        };
        Ok(source)
    }
    /// Terminal-free creation is valid only for the explicitly supplied local host.
    pub fn local(
        exec: std::sync::Arc<dyn nocterm_session::HostExec>,
        cwd: Option<PathBuf>,
    ) -> Result<Self, ProjectParentError> {
        Ok(Self {
            parent: ProjectParentDescriptor::new(HostLocator::Local, "This computer".into())?,
            expected: None,
            cwd,
            exec: Some(exec),
            fs: None,
        })
    }
    pub fn with_resources(
        mut self,
        exec: Option<std::sync::Arc<dyn nocterm_session::HostExec>>,
        fs: Option<std::sync::Arc<dyn nocterm_session::RemoteFs>>,
    ) -> Self {
        self.exec = exec;
        self.fs = fs;
        self
    }
    pub fn executor(&self) -> Option<std::sync::Arc<dyn nocterm_session::HostExec>> {
        self.exec.clone()
    }
    pub fn filesystem(&self) -> Option<std::sync::Arc<dyn nocterm_session::RemoteFs>> {
        self.fs.clone()
    }
    pub fn parent(&self) -> &ProjectParentDescriptor {
        &self.parent
    }
    pub fn expected(&self) -> Option<ExpectedSession> {
        self.expected
    }
    pub fn cwd(&self) -> Option<&std::path::Path> {
        self.cwd.as_deref()
    }
}

/// Optional, narrow human port for selecting the original connection.
pub trait ProjectParentCapture: 'static {
    fn capture(
        &self,
        expected: ExpectedSession,
        cx: &App,
    ) -> Result<ProjectCreationSource, ProjectParentError>;
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum ProjectParentError {
    #[error("The original server connection is no longer open")]
    Disconnected,
    #[error("The server connection changed; reopen workspace creation")]
    StaleSession,
    #[error("Select a server connection to create a workspace")]
    UnavailableParent,
    #[error("The captured server and its capabilities do not match")]
    WrongHost,
    #[error(transparent)]
    InvalidHost(#[from] ProjectValidationError),
}
