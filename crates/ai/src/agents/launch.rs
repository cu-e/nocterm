use std::collections::BTreeMap;

/// How to start one agent process.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AgentLaunch {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub inherit_env: Vec<String>,
}

impl AgentLaunch {
    /// Whether the command or an argument contains `marker`, such as the
    /// package name of an ACP adapter.
    pub fn mentions(&self, marker: &str) -> bool {
        std::iter::once(&self.command)
            .chain(&self.args)
            .any(|part| part.contains(marker))
    }
}

/// Agent ids use letters, numbers, underscore or dash, up to 64 characters.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
