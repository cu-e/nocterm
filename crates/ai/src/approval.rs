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
    pub fn requires_approval(&self, call: &TerminalCall, settings: &ApprovalSettings) -> bool {
        let Some(id) = call.target() else {
            return false;
        };
        if call.writes() {
            settings.terminal_write == ApprovalPolicy::Ask
                && !self.grants.contains(&(id.into(), call.capability()))
        } else {
            settings.terminal_read == ApprovalPolicy::Ask
                && !self.grants.contains(&(id.into(), call.capability()))
        }
    }
}
