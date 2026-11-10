//! The chats other windows hold, which this window's history lists without holding them.
//!
//! The runtime names them; each is observed so that its row follows it — its title, and the
//! spinner while its agent works. Only weak handles are kept: a chat lives as long as its own
//! window keeps it. A burst of changes (a window adopting its saved chats) refreshes the list
//! once.
use super::AgentPanel;
use crate::thread::AgentThread;
use gpui_kit::{App, Context, Entity, EntityId, Subscription, WeakEntity};

#[derive(Default)]
pub(super) struct OtherWindowChats {
    chats: Vec<WeakEntity<AgentThread>>,
    stale: bool,
    _observations: Vec<Subscription>,
}
impl OtherWindowChats {
    /// Lists `chats`, observing each so that its row follows it; whether that changed the list.
    pub(super) fn set(
        &mut self,
        chats: Vec<Entity<AgentThread>>,
        cx: &mut Context<AgentPanel>,
    ) -> bool {
        self.stale = false;
        let same = chats.len() == self.chats.len()
            && chats
                .iter()
                .zip(&self.chats)
                .all(|(chat, known)| chat.entity_id() == known.entity_id());
        if same {
            return false;
        }
        self._observations = chats
            .iter()
            .map(|chat| cx.observe(chat, |panel, _, cx| panel.on_other_window_chat_changed(cx)))
            .collect();
        self.chats = chats.iter().map(Entity::downgrade).collect();
        true
    }
    /// Marks the list for one refresh; `true` the first time since the last.
    pub(super) fn mark_stale(&mut self) -> bool {
        !std::mem::replace(&mut self.stale, true)
    }
    /// The chats with something to list: a chat no one wrote to yet stays in its window.
    pub(super) fn listed(&self, cx: &App) -> Vec<Entity<AgentThread>> {
        self.chats
            .iter()
            .filter_map(WeakEntity::upgrade)
            .filter(|chat| {
                let chat = chat.read(cx);
                !chat.history_entries().is_empty()
                    || chat.archive_summary().is_some_and(|row| row.has_content)
            })
            .collect()
    }
    pub(super) fn find(&self, id: EntityId) -> Option<Entity<AgentThread>> {
        self.chats
            .iter()
            .find(|chat| chat.entity_id() == id)
            .and_then(WeakEntity::upgrade)
    }
}
