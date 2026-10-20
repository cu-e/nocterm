//! Chat membership retains each original entity's observation lifetime.
use crate::thread::AgentThread;
use gpui_kit::{Entity, EntityId, Subscription};

pub(crate) struct ChatSlot {
    thread: Entity<AgentThread>,
    _observation: Subscription,
}
impl ChatSlot {
    pub(crate) fn new(thread: Entity<AgentThread>, observation: Subscription) -> Self {
        Self {
            thread,
            _observation: observation,
        }
    }
    pub(crate) fn thread(&self) -> &Entity<AgentThread> {
        &self.thread
    }
}
#[derive(Default)]
pub(crate) struct ChatList {
    slots: Vec<ChatSlot>,
    active: Option<EntityId>,
}
impl ChatList {
    pub(crate) fn current(&self) -> Option<Entity<AgentThread>> {
        self.active.and_then(|id| self.find(id)).cloned()
    }
    pub(crate) fn find(&self, id: EntityId) -> Option<&Entity<AgentThread>> {
        self.iter().find(|thread| thread.entity_id() == id)
    }
    pub(crate) fn contains(&self, id: EntityId) -> bool {
        self.find(id).is_some()
    }
    pub(crate) fn iter(&self) -> impl ExactSizeIterator<Item = &Entity<AgentThread>> {
        self.slots.iter().map(ChatSlot::thread)
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
    pub(crate) fn select(&mut self, id: EntityId) {
        self.active = self.contains(id).then_some(id);
    }
    pub(crate) fn append(&mut self, slot: ChatSlot) {
        self.slots.push(slot);
    }
    pub(crate) fn prepend(&mut self, slots: Vec<ChatSlot>) {
        self.slots.splice(0..0, slots);
    }
    pub(crate) fn remove_admitted(&mut self, ids: &[EntityId]) {
        self.slots
            .retain(|slot| !ids.contains(&slot.thread.entity_id()));
        if self.active.is_some_and(|id| ids.contains(&id)) {
            self.active = None;
        }
    }
}
