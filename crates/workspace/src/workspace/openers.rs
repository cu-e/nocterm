//! How requests to open a tab become one, so the feature that asks never
//! depends on the feature that answers.

use std::rc::Rc;

use gpui_kit::{Context, EntityId, Window};

use super::{SessionSpec, Workspace};
use crate::{HostKey, LocalTerminalTarget, ProgramSpec};

/// What turns requests into tabs. The host installs one with
/// [`Workspace::set_session_factory`]; the workspace never names the
/// feature behind it.
///
/// A host that cannot open a kind keeps its default, which logs and opens
/// nothing.
pub trait SessionFactory: 'static {
    /// Opens `spec` in a new tab.
    fn open_session(
        &self,
        _workspace: &mut Workspace,
        _spec: SessionSpec,
        _window: &mut Window,
        _cx: &mut Context<Workspace>,
    ) {
        tracing::warn!("no session opener is installed");
    }

    /// Opens `spec` without a tab and returns its item.
    fn open_background(
        &self,
        _workspace: &mut Workspace,
        _spec: SessionSpec,
        _window: &mut Window,
        _cx: &mut Context<Workspace>,
    ) -> Option<EntityId> {
        None
    }

    /// Opens a local shell where `target` says.
    fn open_local(
        &self,
        _workspace: &mut Workspace,
        _target: LocalTerminalTarget,
        _window: &mut Window,
        _cx: &mut Context<Workspace>,
    ) {
    }

    /// Opens `spec`'s program in a new tab.
    fn open_program(
        &self,
        _workspace: &mut Workspace,
        _spec: ProgramSpec,
        _window: &mut Window,
        _cx: &mut Context<Workspace>,
    ) {
        tracing::warn!("no program opener is installed");
    }
}

pub(super) type Factory = Rc<dyn SessionFactory>;

/// Opens nothing, until the host installs a factory.
pub(super) struct NoFactory;
impl SessionFactory for NoFactory {}

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

impl Workspace {
    /// Installs what turns requests into tabs.
    pub fn set_session_factory(&mut self, factory: impl SessionFactory) {
        self.factory = Rc::new(factory);
    }

    /// Installs only what turns a [`SessionSpec`] into a tab, keeping the
    /// rest of the current factory.
    pub fn set_session_opener(
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

    /// Installs only what turns a [`ProgramSpec`] into a tab.
    pub fn set_program_opener(
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

    /// Installs only what opens a local shell.
    pub fn set_local_terminal_opener(
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

    pub(super) fn factory(&self) -> Factory {
        self.factory.clone()
    }

    /// Opens a session in a new tab.
    pub fn open_session(&mut self, spec: SessionSpec, window: &mut Window, cx: &mut Context<Self>) {
        self.show_new_tab_menu(false, cx);
        self.factory().open_session(self, spec, window, cx);
    }

    /// Opens a program in a new tab, grouped with the active tab when that
    /// is a session on the same host, as a container's log is with the
    /// session it was opened from.
    pub fn open_program(&mut self, spec: ProgramSpec, window: &mut Window, cx: &mut Context<Self>) {
        let parent = self
            .active_item
            .and_then(|ix| self.items.get(ix))
            .filter(|open| {
                open.session
                    .as_ref()
                    .is_some_and(|session| HostKey::of(session) == spec.host.key)
            })
            .map(|open| open.handle.item_id());
        let opened_from = self.items.len();
        self.factory().open_program(self, spec, window, cx);
        self.group_program(parent, opened_from, window, cx);
    }
}
