use crate::TerminalCall;
use nocterm_settings::{ApprovalPolicy, ApprovalSettings};
use std::collections::BTreeSet;
#[derive(Default)]
pub struct ApprovalGrants {
    read: BTreeSet<String>,
    write: BTreeSet<String>,
}
impl ApprovalGrants {
    pub fn grant(&mut self, id: &str, write: bool) {
        if write {
            self.write.insert(id.into());
        } else {
            self.read.insert(id.into());
        }
    }
    pub fn revoke(&mut self, id: &str) {
        self.read.remove(id);
        self.write.remove(id);
    }
    pub fn clear(&mut self) {
        self.read.clear();
        self.write.clear();
    }
    pub fn requires_approval(&self, call: &TerminalCall, settings: &ApprovalSettings) -> bool {
        let Some(id) = call.terminal_id() else {
            return false;
        };
        if call.writes() {
            settings.terminal_write == ApprovalPolicy::Ask && !self.write.contains(id)
        } else {
            settings.terminal_read == ApprovalPolicy::Ask && !self.read.contains(id)
        }
    }
}
