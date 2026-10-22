//! Human folder creation captures the original ordinary connection.
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
    expected: ExpectedSession,
    cwd: Option<PathBuf>,
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
            expected,
            cwd,
        };
        Ok(source)
    }
    pub fn parent(&self) -> &ProjectParentDescriptor {
        &self.parent
    }
    pub fn expected(&self) -> ExpectedSession {
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
