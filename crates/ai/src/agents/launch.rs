use nocterm_hubs_model::HubId;
use std::collections::BTreeMap;

/// How to start one agent: a local process, or an agent of a hub.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AgentLaunch {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub inherit_env: Vec<String>,
    /// The hub agent this launch reaches instead of a process; the process fields are empty.
    pub hub: Option<HubAgentRef>,
}

/// An agent that runs on a hub: its chats live there and keep running while no client is
/// open. The hub connector reaches it; no process starts on this computer.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HubAgentRef {
    pub hub: HubId,
    /// The hub's identifier of the agent.
    pub agent: String,
}

impl AgentLaunch {
    /// The launch of agent `agent` of `hub`, named `name`. Its id is stable for the pair, so a
    /// saved chat finds its agent again.
    pub fn hub_agent(hub: HubId, agent: String, name: String) -> Self {
        Self {
            id: hub_agent_id(&hub, &agent),
            name,
            command: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
            inherit_env: Vec::new(),
            hub: Some(HubAgentRef { hub, agent }),
        }
    }

    /// Whether the command or an argument contains `marker`, such as the
    /// package name of an ACP adapter.
    pub fn mentions(&self, marker: &str) -> bool {
        std::iter::once(&self.command)
            .chain(&self.args)
            .any(|part| part.contains(marker))
    }
}

/// How the ids of hub agents begin.
pub const HUB_AGENT_PREFIX: &str = "hub-";

/// [`HUB_AGENT_PREFIX`] and 24 hex digits of a hash of the pair: a valid agent id of at most
/// 64 characters.
fn hub_agent_id(hub: &HubId, agent: &str) -> String {
    use sha2::{Digest as _, Sha256};
    let mut hash = Sha256::new();
    for part in [hub.to_string().as_bytes(), agent.as_bytes()] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    let digest = hash.finalize();
    let mut id = String::from(HUB_AGENT_PREFIX);
    for byte in &digest[..12] {
        use std::fmt::Write as _;
        let _ = write!(id, "{byte:02x}");
    }
    id
}

/// Agent ids use letters, numbers, underscore or dash, up to 64 characters.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
