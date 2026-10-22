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
}
impl StaticProjectParent {
    pub fn new(descriptor: ProjectParentDescriptor, cwd: Option<PathBuf>) -> Self {
        Self { descriptor, cwd }
    }
}
impl ProjectParentCapture for StaticProjectParent {
    fn capture(
        &self,
        expected: ExpectedSession,
        _: &App,
    ) -> Result<ProjectCreationSource, ProjectParentError> {
        ProjectCreationSource::new(self.descriptor.clone(), expected, self.cwd.clone())
    }
}
