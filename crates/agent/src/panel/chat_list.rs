//! Chat membership retains each original entity's observation lifetime.
use crate::thread::AgentThread;
use gpui_kit::{Entity, EntityId, Subscription};

pub(super) struct ChatSlot {
    thread: Entity<AgentThread>,
    _observation: Subscription,
}
impl ChatSlot {
    pub(super) fn new(thread: Entity<AgentThread>, observation: Subscription) -> Self {
        Self {
            thread,
            _observation: observation,
        }
    }
    pub(super) fn thread(&self) -> &Entity<AgentThread> {
        &self.thread
    }
}
#[derive(Default)]
pub(super) struct ChatList {
    slots: Vec<ChatSlot>,
    active: Option<EntityId>,
}
impl ChatList {
    pub(super) fn current(&self) -> Option<Entity<AgentThread>> {
        self.active.and_then(|id| self.find(id)).cloned()
    }
    pub(super) fn find(&self, id: EntityId) -> Option<&Entity<AgentThread>> {
        self.iter().find(|thread| thread.entity_id() == id)
    }
    pub(super) fn contains(&self, id: EntityId) -> bool {
        self.find(id).is_some()
    }
    pub(super) fn iter(&self) -> impl ExactSizeIterator<Item = &Entity<AgentThread>> {
        self.slots.iter().map(ChatSlot::thread)
    }
    pub(super) fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
    pub(super) fn select(&mut self, id: EntityId) {
        self.active = self.contains(id).then_some(id);
    }
    pub(super) fn append(&mut self, slot: ChatSlot) {
        self.slots.push(slot);
    }
    pub(super) fn prepend(&mut self, slots: Vec<ChatSlot>) {
        self.slots.splice(0..0, slots);
    }
    pub(super) fn remove_admitted(&mut self, ids: &[EntityId]) {
        self.slots
            .retain(|slot| !ids.contains(&slot.thread.entity_id()));
        if self.active.is_some_and(|id| ids.contains(&id)) {
            self.active = None;
        }
    }
}
