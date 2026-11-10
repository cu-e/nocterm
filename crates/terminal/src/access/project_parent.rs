//! Human creation captures only the original ordinary server session.
use super::*;
use nocterm_project_model::HostLocator;
use nocterm_workspace::{
    ProjectCreationSource, ProjectParentCapture, ProjectParentDescriptor, ProjectParentError,
};
use std::rc::Rc;

pub(super) struct ParentCapture {
    access: Rc<Access>,
}
impl ParentCapture {
    pub(super) fn new(access: Rc<Access>) -> Self {
        Self { access }
    }
}
impl ProjectParentCapture for ParentCapture {
    fn capture(
        &self,
        expected: ExpectedSession,
        cx: &App,
    ) -> Result<ProjectCreationSource, ProjectParentError> {
        let item = self
            .access
            .item()
            .map_err(|_| ProjectParentError::Disconnected)?;
        let entity = self
            .access
            .terminal
            .upgrade()
            .ok_or(ProjectParentError::Disconnected)?;
        let terminal = entity.read(cx);
        if item != expected.item() || terminal.connection_epoch() != expected.epoch() {
            return Err(ProjectParentError::StaleSession);
        }
        if !terminal.is_connected() {
            return Err(ProjectParentError::Disconnected);
        }
        let spec = terminal.spec();
        let host = if terminal.is_local() {
            HostLocator::Local
        } else {
            let via = spec.address().via;
            match &spec.profile {
                Some(profile) => HostLocator::Ssh {
                    profile: profile.to_string(),
                    target: spec.target.clone(),
                    via,
                },
                None => HostLocator::SshDirect {
                    target: spec.target.clone(),
                    via,
                },
            }
        };
        ProjectCreationSource::new(
            ProjectParentDescriptor::new(host, spec.title.clone())?,
            expected,
            terminal.cwd(),
        )
    }
}
