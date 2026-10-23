//! Explicit original parent identities for public feature tests.
use crate::{
    ExpectedSession, ProjectCreationSource, ProjectParentCapture, ProjectParentDescriptor,
    ProjectParentError,
};
use gpui::App;
use std::path::PathBuf;

pub struct StaticProjectParent {
    descriptor: ProjectParentDescriptor,
    cwd: Option<PathBuf>,
    refusal: Option<ProjectParentError>,
}
impl StaticProjectParent {
    pub fn new(descriptor: ProjectParentDescriptor, cwd: Option<PathBuf>) -> Self {
        Self {
            descriptor,
            cwd,
            refusal: None,
        }
    }
    pub fn with_refusal(mut self, error: ProjectParentError) -> Self {
        self.refusal = Some(error);
        self
    }
}
impl ProjectParentCapture for StaticProjectParent {
    fn capture(
        &self,
        expected: ExpectedSession,
        _: &App,
    ) -> Result<ProjectCreationSource, ProjectParentError> {
        if let Some(error) = &self.refusal {
            return Err(error.clone());
        }
        ProjectCreationSource::new(self.descriptor.clone(), expected, self.cwd.clone())
    }
}
