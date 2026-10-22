use crate::ProjectOpenError;
use gpui_kit::{App, Context, Subscription, Window};
use nocterm_project_model::{Catalog, ProjectId};
use nocterm_project_service::ProjectsHandle;
use nocterm_workspace::{ProjectTerminalOpen, Workspace};
use std::rc::Rc;

#[derive(Clone)]
pub struct Installed {
    model: ProjectsHandle,
    terminals: Rc<dyn ProjectTerminalOpen>,
    resolver: Option<Rc<dyn nocterm_workspace::ProjectTerminalResolve>>,
}
impl Installed {
    pub(crate) fn new(model: ProjectsHandle, terminals: Rc<dyn ProjectTerminalOpen>) -> Self {
        Self {
            model,
            terminals,
            resolver: None,
        }
    }
    pub fn with_terminal_resolver(
        mut self,
        resolver: Rc<dyn nocterm_workspace::ProjectTerminalResolve>,
    ) -> Self {
        self.resolver = Some(resolver);
        self
    }
    pub(crate) fn resolver(&self) -> Option<&Rc<dyn nocterm_workspace::ProjectTerminalResolve>> {
        self.resolver.as_ref()
    }
    pub fn catalog<'a>(&self, cx: &'a App) -> &'a Catalog {
        self.model.catalog(cx)
    }
    pub fn open(
        &self,
        project: ProjectId,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Result<(), ProjectOpenError> {
        crate::opening::open(self, project, workspace, window, cx)
    }
    pub(crate) fn model(&self) -> &ProjectsHandle {
        &self.model
    }
    pub(crate) fn terminals(&self) -> &Rc<dyn ProjectTerminalOpen> {
        &self.terminals
    }
    pub(crate) fn observe(
        &self,
        cx: &mut App,
        changed: impl FnMut(&mut App) + 'static,
    ) -> Subscription {
        self.model.observe(cx, changed)
    }
}
