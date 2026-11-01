use crate::{ApprovalPolicy, ApprovalSettings};
use crate::{TerminalCall, ToolCapability};
use std::collections::BTreeSet;
#[derive(Default)]
pub struct ApprovalGrants {
    grants: BTreeSet<(String, ToolCapability)>,
}
impl ApprovalGrants {
    pub fn grant(&mut self, id: &str, capability: ToolCapability) {
        self.grants.insert((id.into(), capability));
    }
    pub fn revoke(&mut self, id: &str) {
        self.grants.retain(|(target, _)| target != id);
    }
    pub fn clear(&mut self) {
        self.grants.clear();
    }
    /// A call needs a card unless the policy allows it or every terminal it
    /// touches already holds the capability it needs.
    pub fn requires_approval(&self, call: &TerminalCall, settings: &ApprovalSettings) -> bool {
        let mut targets = call.terminals().peekable();
        let server = call.server_id();
        if targets.peek().is_none() && server.is_none() {
            return false;
        }
        let policy = if call.writes() {
            settings.terminal_write
        } else {
            settings.terminal_read
        };
        let capability = call.capability();
        policy == ApprovalPolicy::Ask
            && !targets
                .chain(server)
                .all(|id| self.grants.contains(&(id.into(), capability)))
    }
}
