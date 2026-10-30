//! Who owns an agent, as a person sees it.

use std::cmp::Ordering;

/// The owner area of an agent. An organization carries the name the person knows it by.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AgentScope {
    /// The signed-in person's own agents.
    Personal,
    /// An organization of the account.
    Org {
        /// The hub's identifier of the organization.
        id: String,
        /// The organization's name.
        name: String,
    },
    /// Agents of the whole server, created by its administrator.
    Server,
}

impl AgentScope {
    /// Personal agents first, then organizations by name, then the server.
    pub fn display_order(&self, other: &Self) -> Ordering {
        fn rank(scope: &AgentScope) -> u8 {
            match scope {
                AgentScope::Personal => 0,
                AgentScope::Org { .. } => 1,
                AgentScope::Server => 2,
            }
        }
        match (self, other) {
            (Self::Org { name: a, id: x }, Self::Org { name: b, id: y }) => a
                .to_lowercase()
                .cmp(&b.to_lowercase())
                .then_with(|| a.cmp(b))
                .then_with(|| x.cmp(y)),
            _ => rank(self).cmp(&rank(other)),
        }
    }

    /// Whether the person may edit agents of this scope from the desktop. Organization and
    /// server agents are managed in the web panel.
    pub fn editable_here(&self) -> bool {
        matches!(self, Self::Personal)
    }
}
