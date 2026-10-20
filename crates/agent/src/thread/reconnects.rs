//! Accepted reconnect receipts, removed only by the waiter holding that receipt.
use gpui_kit::EntityId;
use nocterm_workspace_ports::ExpectedSession;
use std::collections::HashMap;
#[derive(Default)]
pub(super) struct Reconnects {
    accepted: HashMap<EntityId, ExpectedSession>,
}
impl Reconnects {
    pub(super) fn contains(&self, item: &EntityId) -> bool {
        self.accepted.contains_key(item)
    }
    pub(super) fn receipt(&self, expected: ExpectedSession) -> Option<ExpectedSession> {
        self.accepted
            .get(&expected.item())
            .copied()
            .filter(|receipt| *receipt == expected)
    }
    pub(super) fn insert(&mut self, expected: ExpectedSession) {
        self.accepted.insert(expected.item(), expected);
    }
    pub(super) fn remove(&mut self, expected: ExpectedSession) {
        if self.accepted.get(&expected.item()) == Some(&expected) {
            self.accepted.remove(&expected.item());
        }
    }
}
