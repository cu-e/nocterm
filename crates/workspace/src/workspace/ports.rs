//! Explicit collaborators and their installation; invocation stays at composition.
use super::openers::{Factory, NoFactory, SessionFactory};
use super::{SessionSpec, Workspace};
use crate::{ConnectionDirectory, LocalTerminalTarget, ProgramSpec};
use gpui_kit::{Context, EntityId, Window};
use std::rc::Rc;

pub(super) struct WorkspacePorts {
    factory: Factory,
    directory: Option<Rc<dyn ConnectionDirectory>>,
}
impl Default for WorkspacePorts {
    fn default() -> Self {
        Self {
            factory: Rc::new(NoFactory),
            directory: None,
        }
    }
}
impl WorkspacePorts {
    pub(super) fn factory(&self) -> Factory {
        self.factory.clone()
    }
    pub(super) fn install(&mut self, factory: impl SessionFactory) {
        self.factory = Rc::new(factory);
    }
    pub(super) fn set_directory(&mut self, directory: Rc<dyn ConnectionDirectory>) {
        self.directory = Some(directory);
    }
    pub(super) fn directory(&self) -> Option<Rc<dyn ConnectionDirectory>> {
        self.directory.clone()
    }
    pub(super) fn override_session(
        &mut self,
        opener: impl Fn(&mut Workspace, SessionSpec, &mut Window, &mut Context<Workspace>) + 'static,
    ) {
        self.factory = Rc::new(Override {
            base: self.factory.clone(),
            session: Some(Box::new(opener)),
            program: None,
            local: None,
        });
    }
    pub(super) fn override_program(
        &mut self,
        opener: impl Fn(&mut Workspace, ProgramSpec, &mut Window, &mut Context<Workspace>) + 'static,
    ) {
        self.factory = Rc::new(Override {
            base: self.factory.clone(),
            session: None,
            program: Some(Box::new(opener)),
            local: None,
        });
    }
    pub(super) fn override_local(
        &mut self,
        opener: impl Fn(&mut Workspace, LocalTerminalTarget, &mut Window, &mut Context<Workspace>)
        + 'static,
    ) {
        self.factory = Rc::new(Override {
            base: self.factory.clone(),
            session: None,
            program: None,
            local: Some(Box::new(opener)),
        });
    }
}
type Open<T> = Box<dyn Fn(&mut Workspace, T, &mut Window, &mut Context<Workspace>)>;

/// One kind opened by a closure, the rest by the factory beneath; for tests
/// and hosts that open one kind.
struct Override {
    base: Factory,
    session: Option<Open<SessionSpec>>,
    program: Option<Open<ProgramSpec>>,
    local: Option<Open<LocalTerminalTarget>>,
}

impl SessionFactory for Override {
    fn open_session(
        &self,
        workspace: &mut Workspace,
        spec: SessionSpec,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        match &self.session {
            Some(open) => open(workspace, spec, window, cx),
            None => self.base.open_session(workspace, spec, window, cx),
        }
    }
    fn open_background(
        &self,
        workspace: &mut Workspace,
        spec: SessionSpec,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Option<EntityId> {
        self.base.open_background(workspace, spec, window, cx)
    }
    fn open_local(
        &self,
        workspace: &mut Workspace,
        target: LocalTerminalTarget,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        match &self.local {
            Some(open) => open(workspace, target, window, cx),
            None => self.base.open_local(workspace, target, window, cx),
        }
    }
    fn open_program(
        &self,
        workspace: &mut Workspace,
        spec: ProgramSpec,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        match &self.program {
            Some(open) => open(workspace, spec, window, cx),
            None => self.base.open_program(workspace, spec, window, cx),
        }
    }
}
