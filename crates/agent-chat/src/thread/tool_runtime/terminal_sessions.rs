//! Background membership, secret prompts and reconnect receipts retain original Item authority.
use super::super::{SignInWait, reconnects::Reconnects};
use gpui::EntityId;
use nocterm_workspace_ports::ExpectedSession;
#[derive(Default)]
pub(super) struct TerminalSessions {
    background: Vec<EntityId>,
    sign_ins: Vec<SignInWait>,
    reconnects: Reconnects,
}
impl TerminalSessions {
    pub(super) fn background(&self) -> &[EntityId] {
        &self.background
    }
    pub(super) fn remember_opened(&mut self, item: EntityId) {
        self.background.push(item);
    }
    pub(super) fn forget_opened(&mut self, item: EntityId) {
        self.background.retain(|current| *current != item);
    }
    pub(super) fn take_close_effects(&mut self) -> Vec<EntityId> {
        std::mem::take(&mut self.background)
    }
    pub(super) fn prune_background(&mut self, retained: &[EntityId]) -> Vec<EntityId> {
        let (keep, close) = self
            .background
            .drain(..)
            .partition(|id| retained.contains(id));
        self.background = keep;
        close
    }
    pub(super) fn sign_ins(&self) -> &[SignInWait] {
        &self.sign_ins
    }
    pub(super) fn observe_prompt(&mut self, item: EntityId, wait: Option<SignInWait>) -> bool {
        let current = self.sign_ins.iter().position(|wait| wait.item() == item);
        match (current, wait) {
            (Some(index), Some(wait)) => {
                let old = &self.sign_ins[index];
                if old.prompt() == wait.prompt() && old.vault() == wait.vault() {
                    return false;
                }
                self.sign_ins[index] = wait;
                true
            }
            (None, Some(wait)) => {
                self.sign_ins.push(wait);
                true
            }
            (Some(index), None) => {
                self.sign_ins.remove(index);
                true
            }
            (None, None) => false,
        }
    }
    pub(super) fn clear_wait(&mut self, expected: ExpectedSession) {
        self.sign_ins.retain(|wait| {
            wait.item() != expected.item() || wait.prompt().token.expected() != expected
        });
    }
    pub(super) fn reconnecting(&self, item: &EntityId) -> bool {
        self.reconnects.contains(item)
    }
    pub(super) fn reconnect_receipt(&self, expected: ExpectedSession) -> Option<ExpectedSession> {
        self.reconnects.receipt(expected)
    }
    pub(super) fn accept_reconnect(&mut self, expected: ExpectedSession) {
        self.reconnects.insert(expected);
    }
    pub(super) fn finish_reconnect(&mut self, expected: ExpectedSession) {
        self.reconnects.remove(expected);
    }
}
