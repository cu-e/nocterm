//! Off-dock membership retains the original Item and subscription lifetimes.
use super::*;
use tabs::TabEntry;

#[derive(Default)]
pub(super) struct BackgroundSessions {
    entries: Vec<TabEntry>,
}
impl BackgroundSessions {
    pub(super) fn ids(&self) -> Vec<EntityId> {
        self.entries.iter().map(TabEntry::id).collect()
    }
    pub(super) fn item_handle(&self, id: EntityId) -> Option<Rc<dyn ItemHandle>> {
        self.entries
            .iter()
            .find(|entry| entry.id() == id)
            .map(TabEntry::handle)
    }
    pub(super) fn contains(&self, id: EntityId) -> bool {
        self.entries.iter().any(|entry| entry.id() == id)
    }
    pub(super) fn insert(&mut self, entry: TabEntry) -> bool {
        if self.contains(entry.id()) {
            return false;
        }
        self.entries.push(entry);
        true
    }
    pub(super) fn changed(
        &mut self,
        id: EntityId,
        session: Option<SessionContext>,
        cx: &mut App,
    ) -> bool {
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.id() == id) else {
            return false;
        };
        entry.refresh(session, cx);
        true
    }
    pub(super) fn take(&mut self, id: EntityId) -> Option<TabEntry> {
        let index = self.entries.iter().position(|entry| entry.id() == id)?;
        Some(self.entries.remove(index))
    }
    pub(super) fn terminals(&self, cx: &App) -> Vec<crate::TerminalEntry> {
        self.entries
            .iter()
            .filter_map(|entry| {
                let item = entry.handle();
                Some(crate::TerminalEntry {
                    item: item.item_id(),
                    capabilities: item.terminal_capabilities(cx)?,
                    title: item.tab_title(cx),
                    active: false,
                    bottom: false,
                    background: true,
                })
            })
            .collect()
    }
}
