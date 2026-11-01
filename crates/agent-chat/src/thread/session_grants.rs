//! Reusable target grants expire when the observed session changes.
use nocterm_ai::{ApprovalSettings, TerminalCall, ToolCapability, approval::ApprovalGrants};
use nocterm_workspace_ports::ExpectedSession;
use std::collections::HashMap;
#[derive(Default)]
pub(super) struct SessionGrants {
    grants: ApprovalGrants,
    sessions: HashMap<String, ExpectedSession>,
}
impl SessionGrants {
    pub(super) fn observe(&mut self, call: &TerminalCall, expected: Option<ExpectedSession>) {
        if let Some(id) = call.terminal_id() {
            self.observe_session(id, expected);
        }
    }
    /// The source terminal of a transfer expires its grants the same way.
    pub(super) fn observe_source(
        &mut self,
        call: &TerminalCall,
        expected: Option<ExpectedSession>,
    ) {
        if let Some(id) = call.source_terminal_id() {
            self.observe_session(id, expected);
        }
    }
    fn observe_session(&mut self, id: &str, expected: Option<ExpectedSession>) {
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
