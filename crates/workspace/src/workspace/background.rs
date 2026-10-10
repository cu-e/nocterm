//! Background membership owns its typed Item and subscriptions until promotion/close.
use super::*;
use bindings::{PreparedTab, WorkspaceBindings};

pub(super) trait BackgroundRecord {
    fn handle(&self) -> &dyn ItemHandle;
    fn close(self: Box<Self>, window: &mut Window, cx: &mut App);
    fn into_tab(
        self: Box<Self>,
        bindings: &WorkspaceBindings,
        origin: Rc<Cell<Option<EntityId>>>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> PreparedTab;
}

struct BackgroundEntry<T: Item> {
    item: Entity<T>,
    handle: Rc<dyn ItemHandle>,
    subscription: Subscription,
}
impl<T: Item> BackgroundRecord for BackgroundEntry<T> {
    fn handle(&self) -> &dyn ItemHandle {
        self.handle.as_ref()
    }
    fn close(self: Box<Self>, window: &mut Window, cx: &mut App) {
        let Self {
            item,
            handle,
            subscription,
        } = *self;
        drop(subscription);
        handle.close(window, cx);
        drop(item);
    }
    fn into_tab(
        self: Box<Self>,
        bindings: &WorkspaceBindings,
        origin: Rc<Cell<Option<EntityId>>>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> PreparedTab {
        let Self {
            item,
            handle,
            subscription,
        } = *self;
        drop(subscription);
        // Keep the original Item strongly owned across removing the old bindings.
        let prepared = bindings.prepare(item, None, origin, window, cx);
        drop(handle);
        prepared
    }
}

#[derive(Default)]
pub(super) struct BackgroundSessions {
    entries: Vec<Box<dyn BackgroundRecord>>,
}
impl BackgroundSessions {
    pub(super) fn contains(&self, id: EntityId) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.handle().item_id() == id)
    }
    pub(super) fn insert<T: Item>(&mut self, item: Entity<T>, subscription: Subscription) -> bool {
        if self.contains(item.entity_id()) {
            return false;
        }
        self.entries.push(Box::new(BackgroundEntry {
            handle: Rc::new(item.clone()),
            item,
            subscription,
        }));
        true
    }
    pub(super) fn take(&mut self, id: EntityId) -> Option<Box<dyn BackgroundRecord>> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.handle().item_id() == id)?;
        Some(self.entries.remove(index))
    }
    pub(super) fn terminals(&self, cx: &App) -> Vec<crate::TerminalEntry> {
        self.entries
            .iter()
            .filter_map(|entry| {
                let item = entry.handle();
                Some(crate::TerminalEntry {
                    item: item.item_id(),
                    access: item.terminal_access(cx)?,
                    title: item.tab_title(cx),
                    active: false,
                    bottom: false,
                    background: true,
                })
            })
            .collect()
    }
}
