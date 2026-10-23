//! Records ordinary folder terminal opens through the consumer port.
use crate::{
    ContextSelectionError, ContextSelectionReceipt, ExpectedSession, ItemHandle,
    ProjectTerminalOpen, ProjectTerminalSession, ProjectTerminalTarget,
};
use gpui_kit::{App, Window};
use nocterm_project_model::{ProjectTarget, RootPath};
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
        parent: Rc<dyn ItemHandle>,
        expected: ExpectedSession,
        location: &ProjectTarget,
        cx: &App,
    ) -> Result<Rc<dyn ProjectTerminalSession>, ContextSelectionError> {
        let info = parent
            .terminal_capabilities(cx)
            .and_then(|ports| ports.state().info(cx))
            .ok_or(ContextSelectionError::ParentUnavailable)?;
        if info.expected != expected || parent.item_id() != expected.item() {
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
        target.validate(window, cx)?;
        let root = self
            .root
            .as_ref()
            .ok_or(ContextSelectionError::Unavailable)?;
        self.attempts
            .borrow_mut()
            .push((target.selection(), root.clone()));
        if let Some(spec) = &self.session {
            let item = super::SessionProbe::from_app(spec.clone(), true, None, cx);
            if let Some(executor) = &self.executor {
                item.update(cx, |item, cx| item.set_executor(executor.clone(), cx));
            }
            target
                .surface()
                .add_item(target.selection(), item, window, cx)?;
        }
        Ok(())
    }
}

impl crate::ProjectTerminalResolve for ProjectTerminalProbe {
    fn resolve(
        &self,
        location: &ProjectTarget,
        _: &App,
    ) -> Result<Rc<dyn ProjectTerminalSession>, crate::ProjectConnectionError> {
        use nocterm_project_model::HostLocator;
        let (host, root) = location
            .host_folder()
            .ok_or(crate::ProjectConnectionError::Unavailable)?;
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
            _ => return Err(crate::ProjectConnectionError::Unavailable),
        };
        if !matches {
            return Err(crate::ProjectConnectionError::ChangedTargetOrRoute);
        }
        let mut captured = self.clone();
        captured.root = Some(root.clone());
        Ok(Rc::new(captured))
    }
}
