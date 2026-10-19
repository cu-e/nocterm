//! How requests to open a tab become one, so the feature that asks never
//! depends on the feature that answers.

use std::rc::Rc;

use gpui_kit::{Context, EntityId, Window};

use super::{SessionSpec, Workspace};
use crate::{LocalTerminalTarget, ProgramSpec};

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
