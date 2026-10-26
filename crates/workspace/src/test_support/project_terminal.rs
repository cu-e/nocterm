//! Records ordinary folder terminal opens through the consumer port.
use crate::{ContextSelectionReceipt, ExpectedSession};
use gpui_kit::{AnyView, App, Window};
use nocterm_project_model::{ProjectTarget, RootPath};
use nocterm_workspace_ports::TerminalCapabilities;
use nocterm_workspace_ports::{
    ContextSelectionError, ProjectConnectionError, ProjectTerminalOpen, ProjectTerminalResolve,
    ProjectTerminalSession, ProjectTerminalTarget,
};
use std::{cell::RefCell, rc::Rc};
#[derive(Clone, Default)]
pub struct ProjectTerminalProbe {
    attempts: Rc<RefCell<Vec<(ContextSelectionReceipt, RootPath)>>>,
    session: Option<crate::SessionSpec>,
    local: bool,
    resolutions: Rc<RefCell<Vec<nocterm_project_model::HostLocator>>>,
    root: Option<RootPath>,
    executor: Option<std::sync::Arc<dyn nocterm_session::HostExec>>,
}
impl ProjectTerminalProbe {
    pub fn with_executor(
        mut self,
        executor: std::sync::Arc<dyn nocterm_session::HostExec>,
    ) -> Self {
        self.executor = Some(executor);
        self
    }
    pub fn with_sessions(spec: crate::SessionSpec) -> Self {
        Self {
            session: Some(spec),
            ..Self::default()
        }
    }
    pub fn with_local_sessions(spec: crate::SessionSpec) -> Self {
        Self {
            local: true,
            ..Self::with_sessions(spec)
        }
    }
    pub fn resolutions(&self) -> Vec<nocterm_project_model::HostLocator> {
        self.resolutions.borrow().clone()
    }
    pub fn attempts(&self) -> Vec<(ContextSelectionReceipt, RootPath)> {
        self.attempts.borrow().clone()
    }
}
impl ProjectTerminalOpen for ProjectTerminalProbe {
    fn capture(
        &self,
        parent: AnyView,
        expected: ExpectedSession,
        capabilities: TerminalCapabilities,
        location: &ProjectTarget,
        cx: &App,
    ) -> Result<Rc<dyn ProjectTerminalSession>, ContextSelectionError> {
        let info = capabilities
            .state()
            .info(cx)
            .ok_or(ContextSelectionError::ParentUnavailable)?;
        if info.expected != expected || parent.entity_id() != expected.item() {
            return Err(crate::ProjectParentError::StaleSession.into());
        }
        if info.status != crate::TerminalStatus::Connected {
            return Err(ContextSelectionError::ParentUnavailable);
        }
        let (_, root) = location
            .host_folder()
            .ok_or(ContextSelectionError::Unavailable)?;
        let mut captured = self.clone();
        captured.root = Some(root.clone());
        Ok(Rc::new(captured))
    }
}
impl ProjectTerminalSession for ProjectTerminalProbe {
    fn open(
        &self,
        target: ProjectTerminalTarget,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), ContextSelectionError> {
        crate::validate_project_terminal_target(&target, window, cx)?;
        let root = self
            .root
            .as_ref()
            .ok_or(ContextSelectionError::Unavailable)?;
        self.attempts.borrow_mut().push((
            crate::workspace::project_terminal_selection(&target, cx)?,
            root.clone(),
        ));
        if let Some(spec) = &self.session {
            let item = super::SessionProbe::from_app(spec.clone(), true, None, cx);
            if let Some(executor) = &self.executor {
                item.update(cx, |item, cx| item.set_executor(executor.clone(), cx));
            }
            crate::admit_project_terminal(&target, item, window, cx)?;
        }
        Ok(())
    }
}

impl ProjectTerminalResolve for ProjectTerminalProbe {
    fn resolve(
        &self,
        location: &ProjectTarget,
        _: &App,
    ) -> Result<Rc<dyn ProjectTerminalSession>, ProjectConnectionError> {
        use nocterm_project_model::HostLocator;
        let (host, root) = location
            .host_folder()
            .ok_or(ProjectConnectionError::Unavailable)?;
        self.resolutions.borrow_mut().push(host.clone());
        host.validate()?;
        let matches = match (host, self.session.as_ref()) {
            (HostLocator::Local, Some(_)) => self.local,
            (HostLocator::Ssh { profile, .. }, Some(spec)) => {
                !self.local
                    && spec.profile.as_deref() == Some(profile.as_str())
                    && host.address().as_ref() == Some(&spec.address())
            }
            (HostLocator::SshDirect { .. }, Some(spec)) => {
                !self.local && host.address().as_ref() == Some(&spec.address())
            }
            _ => return Err(ProjectConnectionError::Unavailable),
        };
        if !matches {
            return Err(ProjectConnectionError::ChangedTargetOrRoute);
        }
        let mut captured = self.clone();
        captured.root = Some(root.clone());
        Ok(Rc::new(captured))
    }
}

// A provider-shaped foreign target exercises checked shell destination erasure.
impl nocterm_workspace_ports::ProjectTerminalDestination for ProjectTerminalProbe {
    fn validate(&self, _: &Window, _: &App) -> Result<(), ContextSelectionError> {
        Ok(())
    }
}
