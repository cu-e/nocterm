//! One original window claim and captured ordinary terminal session provider.
use crate::{Installed, ProjectOpenError};
use gpui_kit::App;
use nocterm_project_service::ProjectClaim;
use nocterm_workspace::{ContextSelectionError, ProjectTerminalSession};
use std::rc::Rc;

pub(crate) struct ProjectLifetime {
    claim: ProjectClaim,
    session: Result<Rc<dyn ProjectTerminalSession>, ProjectOpenError>,
    first_requested: bool,
    closed: bool,
    error: Option<ProjectOpenError>,
}
impl ProjectLifetime {
    pub(crate) fn new(
        claim: ProjectClaim,
        session: Result<Rc<dyn ProjectTerminalSession>, ProjectOpenError>,
    ) -> Self {
        Self {
            claim,
            session,
            first_requested: false,
            closed: false,
            error: None,
        }
    }
    pub(crate) fn claim(&self) -> ProjectClaim {
        self.claim
    }
    pub(crate) fn start_first(&mut self) -> bool {
        if self.closed || self.first_requested || self.session.is_err() {
            return false;
        }
        self.first_requested = true;
        true
    }
    pub(crate) fn session(&self) -> Result<Rc<dyn ProjectTerminalSession>, ProjectOpenError> {
        if self.closed {
            return Err(ContextSelectionError::StaleSurface.into());
        }
        self.session.clone()
    }
    pub(crate) fn failed(&mut self, error: ProjectOpenError) {
        self.error = Some(error);
    }
    pub(crate) fn can_retry(&self) -> bool {
        !self.closed && (self.session.is_err() || self.error.is_some())
    }
    pub(crate) fn retry(
        &mut self,
        session: Result<Rc<dyn ProjectTerminalSession>, ProjectOpenError>,
    ) {
        self.session = session;
        self.error = None;
    }
    pub(crate) fn message(&self) -> Option<String> {
        self.error
            .as_ref()
            .or_else(|| self.session.as_ref().err())
            .map(ToString::to_string)
    }
    pub(crate) fn close(&mut self, installed: &Installed, cx: &mut App) {
        if self.closed {
            return;
        }
        self.closed = true;
        let _ = installed.model().release_claim(self.claim, cx);
    }
}
