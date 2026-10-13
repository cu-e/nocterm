//! Reusable target grants expire when the observed session changes.
use nocterm_ai::{ApprovalSettings, TerminalCall, ToolCapability, approval::ApprovalGrants};
use nocterm_workspace::ExpectedSession;
use std::collections::HashMap;
#[derive(Default)]
pub(super) struct SessionGrants {
    grants: ApprovalGrants,
    sessions: HashMap<String, ExpectedSession>,
}
impl SessionGrants {
    pub(super) fn observe(&mut self, call: &TerminalCall, expected: Option<ExpectedSession>) {
        let Some(id) = call.terminal_id() else {
            return;
        };
        if self.sessions.get(id).copied() != expected {
            self.grants.revoke(id);
            self.sessions.remove(id);
            if let Some(expected) = expected {
                self.sessions.insert(id.to_owned(), expected);
            }
        }
    }
    pub(super) fn requires_approval(&self, call: &TerminalCall, policy: &ApprovalSettings) -> bool {
        self.grants.requires_approval(call, policy)
    }
    pub(super) fn grant(&mut self, id: &str, capability: ToolCapability) {
        self.grants.grant(id, capability);
    }
    pub(super) fn clear(&mut self) {
        self.grants.clear();
        self.sessions.clear();
    }
}
