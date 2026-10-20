//! Chat membership retains each original entity's observation lifetime.
use crate::thread::AgentThread;
use gpui::{Entity, EntityId, Subscription};

pub struct ChatSlot {
    thread: Entity<AgentThread>,
    _observation: Subscription,
}
impl ChatSlot {
    pub fn new(thread: Entity<AgentThread>, observation: Subscription) -> Self {
        Self {
            thread,
            _observation: observation,
        }
    }
    pub fn thread(&self) -> &Entity<AgentThread> {
        &self.thread
    }
}
#[derive(Default)]
pub struct ChatList {
    slots: Vec<ChatSlot>,
    active: Option<EntityId>,
}
impl ChatList {
    pub fn current(&self) -> Option<Entity<AgentThread>> {
        self.active.and_then(|id| self.find(id)).cloned()
    }
    pub fn find(&self, id: EntityId) -> Option<&Entity<AgentThread>> {
        self.iter().find(|thread| thread.entity_id() == id)
    }
    pub fn contains(&self, id: EntityId) -> bool {
        self.find(id).is_some()
    }
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Entity<AgentThread>> {
        self.slots.iter().map(ChatSlot::thread)
    }
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
    pub fn select(&mut self, id: EntityId) {
        self.active = self.contains(id).then_some(id);
    }
    pub fn append(&mut self, slot: ChatSlot) {
        self.slots.push(slot);
    }
    pub fn prepend(&mut self, slots: Vec<ChatSlot>) {
        self.slots.splice(0..0, slots);
    }
    pub fn remove_admitted(&mut self, ids: &[EntityId]) {
        self.slots
            .retain(|slot| !ids.contains(&slot.thread.entity_id()));
        if self.active.is_some_and(|id| ids.contains(&id)) {
            self.active = None;
        }
    }
}
