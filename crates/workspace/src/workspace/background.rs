//! Sessions without a tab.
//!
//! An agent working on a saved server nobody has open gets a session in the
//! background: it connects and runs like a tab, but is not shown. The user
//! can bring it into a tab at any time; one that needs the user (a password
//! or host key prompt) is brought into a tab by its owner.
use gpui_kit::{Context, Entity, EntityId, Subscription, Window};
use std::rc::Rc;

use super::{SessionSpec, Workspace, WorkspaceEvent};
use crate::{Item, ItemEvent, ItemHandle};

type Show = Box<dyn FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>)>;
pub(super) type BackgroundOpener =
    Rc<dyn Fn(&mut Workspace, SessionSpec, &mut Window, &mut Context<Workspace>) -> EntityId>;

pub(super) struct BackgroundItem {
    pub(super) handle: Rc<dyn ItemHandle>,
    /// Moves the item into a tab; typed, because tabs subscribe to the item.
    show: Option<Show>,
    _subscription: Subscription,
}

impl Workspace {
    /// Installs what turns a [`SessionSpec`] into a session without a tab.
    pub fn set_background_session_opener(
        &mut self,
        opener: impl Fn(&mut Workspace, SessionSpec, &mut Window, &mut Context<Workspace>) -> EntityId
        + 'static,
    ) {
        self.background_opener = Some(Rc::new(opener));
    }

    /// Opens a session that is not shown in a tab, and returns its item.
    pub fn open_background_session(
        &mut self,
        spec: SessionSpec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<EntityId> {
        let opener = self.background_opener.clone()?;
        Some(opener(self, spec, window, cx))
    }

    /// Keeps `item` running without a tab.
    pub fn add_background_item<T: Item>(
        &mut self,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> EntityId {
        let id = item.entity_id();
        let subscription =
            cx.subscribe_in(
                &item,
                window,
                move |this, _, event, window, cx| match event {
                    ItemEvent::Changed => cx.emit(WorkspaceEvent::ItemsChanged),
                    ItemEvent::CloseRequested => this.close_background_session(id, window, cx),
                },
            );
        let typed = item.clone();
        self.background.push(BackgroundItem {
            handle: Rc::new(item),
            show: Some(Box::new(move |workspace, window, cx| {
                workspace.add_item(typed, window, cx)
            })),
            _subscription: subscription,
        });
        cx.emit(WorkspaceEvent::ItemsChanged);
        cx.notify();
        id
    }

    /// Whether `id` is a session without a tab.
    pub fn is_background(&self, id: EntityId) -> bool {
        self.background
            .iter()
            .any(|item| item.handle.item_id() == id)
    }

    /// Moves a background session into a tab, keeping its connection.
    pub fn show_background_session(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(index) = self
            .background
            .iter()
            .position(|item| item.handle.item_id() == id)
        else {
            return false;
        };
        let mut item = self.background.remove(index);
        if let Some(show) = item.show.take() {
            show(self, window, cx);
        }
        true
    }

    /// Ends a background session.
    pub fn close_background_session(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self
            .background
            .iter()
            .position(|item| item.handle.item_id() == id)
        {
            let item = self.background.remove(index);
            item.handle.close(window, cx);
            cx.emit(WorkspaceEvent::ItemsChanged);
            cx.notify();
        }
    }

    pub(super) fn background_entries(&self, cx: &gpui_kit::App) -> Vec<crate::TerminalEntry> {
        self.background
            .iter()
            .filter_map(|item| {
                Some(crate::TerminalEntry {
                    item: item.handle.item_id(),
                    access: item.handle.terminal_access(cx)?,
                    title: item.handle.tab_title(cx),
                    active: false,
                    bottom: false,
                    background: true,
                })
            })
            .collect()
    }
}
