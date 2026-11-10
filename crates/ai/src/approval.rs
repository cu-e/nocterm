use crate::{ApprovalPolicy, ApprovalSettings};
use crate::{TerminalCall, ToolCapability};
use std::collections::BTreeSet;
#[derive(Default)]
pub struct ApprovalGrants {
    grants: BTreeSet<(String, ToolCapability)>,
    /// Transfer grants belong to an unordered pair of terminals, so approving
    /// `a`/`b` and `c`/`d` does not approve `a`/`d`. A same-terminal transfer is `(a, a)`.
    transfer_pairs: BTreeSet<(String, String)>,
}
/// The unordered terminal pair a transfer moves files between.
fn transfer_pair(call: &TerminalCall) -> Option<(String, String)> {
    let (source, destination) = (call.source_terminal_id()?, call.terminal_id()?);
    let (first, second) = if source <= destination {
        (source, destination)
    } else {
        (destination, source)
    };
    Some((first.into(), second.into()))
}
impl ApprovalGrants {
    pub fn grant(&mut self, id: &str, capability: ToolCapability) {
        self.grants.insert((id.into(), capability));
    }
    /// Records what a card approval of `call` covers: a transfer grants its terminal
    /// pair, every other call grants its capability on each terminal it touches.
    pub fn grant_call(&mut self, call: &TerminalCall) {
        if let Some(pair) = transfer_pair(call) {
            self.transfer_pairs.insert(pair);
            return;
        }
        for id in call.terminals().chain(call.server_id()) {
            self.grant(id, call.capability());
        }
    }
    pub fn revoke(&mut self, id: &str) {
        self.grants.retain(|(target, _)| target != id);
        self.transfer_pairs
            .retain(|(first, second)| first != id && second != id);
    }
    pub fn clear(&mut self) {
        self.grants.clear();
        self.transfer_pairs.clear();
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
        let covered = match transfer_pair(call) {
            Some(pair) => self.transfer_pairs.contains(&pair),
            None => targets
                .chain(server)
                .all(|id| self.grants.contains(&(id.into(), capability))),
        };
        policy == ApprovalPolicy::Ask && !covered
    }
}
