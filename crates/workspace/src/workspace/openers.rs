//! How requests to open a tab become one, so the feature that asks never
//! depends on the feature that answers.

use std::rc::Rc;

use gpui_kit::{Context, Window};

use super::{SessionSpec, Workspace};
use crate::{HostKey, ProgramSpec};

pub(super) type ProgramOpener =
    Rc<dyn Fn(&mut Workspace, ProgramSpec, &mut Window, &mut Context<Workspace>)>;

impl Workspace {
    /// Installs what turns a [`SessionSpec`] into a tab.
    pub fn set_session_opener(
        &mut self,
        opener: impl Fn(&mut Workspace, SessionSpec, &mut Window, &mut Context<Workspace>) + 'static,
    ) {
        self.session_opener = Some(Rc::new(opener));
    }

    /// Opens a session in a new tab.
    pub fn open_session(&mut self, spec: SessionSpec, window: &mut Window, cx: &mut Context<Self>) {
        self.show_new_tab_menu(false, cx);
        match self.session_opener.clone() {
            Some(opener) => opener(self, spec, window, cx),
            None => tracing::warn!("no session opener is installed"),
        }
    }

    /// Installs what turns a [`ProgramSpec`] into a tab.
    pub fn set_program_opener(
        &mut self,
        opener: impl Fn(&mut Workspace, ProgramSpec, &mut Window, &mut Context<Workspace>) + 'static,
    ) {
        self.program_opener = Some(Rc::new(opener));
    }

    /// Opens a program in a new tab, grouped with the active tab when that
    /// is a session on the same host, as a container's log is with the
    /// session it was opened from.
    pub fn open_program(&mut self, spec: ProgramSpec, window: &mut Window, cx: &mut Context<Self>) {
        let Some(opener) = self.program_opener.clone() else {
            tracing::warn!("no program opener is installed");
            return;
        };
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
        opener(self, spec, window, cx);
        self.group_program(parent, opened_from, window, cx);
    }
}
