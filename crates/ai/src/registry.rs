use nocterm_settings::AiSettings;
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AgentLaunch {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub inherit_env: Vec<String>,
}
#[derive(Clone, Debug, Default)]
pub struct AgentRegistry {
    pub agents: BTreeMap<String, AgentLaunch>,
    pub warnings: Vec<String>,
}
impl AgentRegistry {
    pub fn new(settings: &AiSettings) -> Self {
        let mut result = Self::default();
        for (id, name, command, args, inherit) in [
            (
                "claude",
                "Claude",
                "npx",
                vec!["-y", "@agentclientprotocol/claude-agent-acp@0.85.1"],
                vec![
                    "ANTHROPIC_API_KEY",
                    "ANTHROPIC_BASE_URL",
                    "CLAUDE_CONFIG_DIR",
                ],
            ),
            (
                "codex",
                "Codex",
                "npx",
                vec!["-y", "@agentclientprotocol/codex-acp@2.1.1"],
                vec!["OPENAI_API_KEY", "OPENAI_BASE_URL", "CODEX_HOME"],
            ),
            (
                "hermes",
                "Hermes",
                "hermes",
                vec!["acp"],
                vec!["HERMES_HOME"],
            ),
        ] {
            result.agents.insert(
                id.into(),
                AgentLaunch {
                    id: id.into(),
                    name: name.into(),
                    command: command.into(),
                    args: args.into_iter().map(str::to_owned).collect(),
                    env: BTreeMap::new(),
                    inherit_env: inherit.into_iter().map(str::to_owned).collect(),
                },
            );
        }
        for (id, setting) in &settings.agents {
            if !valid_id(id)
                || setting
                    .env
                    .keys()
                    .any(|key| crate::env::credential_override(key))
                || setting
                    .env
                    .keys()
                    .chain(setting.inherit_env.iter())
                    .any(|key| !crate::env::valid_name(key))
            {
                result
                    .warnings
                    .push(format!("Invalid agent configuration: {id}"));
                result.agents.remove(id);
                continue;
            }
            if !setting.enabled {
                result.agents.remove(id);
                continue;
            }
            let entry = result
                .agents
                .entry(id.clone())
                .or_insert_with(|| AgentLaunch {
                    id: id.clone(),
                    name: id.clone(),
                    command: String::new(),
                    args: Vec::new(),
                    env: BTreeMap::new(),
                    inherit_env: Vec::new(),
                });
            if let Some(name) = &setting.name {
                entry.name = name.clone();
            }
            if let Some(command) = &setting.command {
                entry.command = command.clone();
            }
            if let Some(args) = &setting.args {
                entry.args = args.clone();
            }
            entry.env.extend(setting.env.clone());
            entry.inherit_env.extend(setting.inherit_env.clone());
            if entry.command.trim().is_empty()
                || entry.command.contains('\0')
                || entry.args.iter().any(|s| s.contains('\0'))
            {
                result
                    .warnings
                    .push(format!("Missing or invalid command: {id}"));
                result.agents.remove(id);
            }
        }
        if !settings.enabled {
            result.agents.clear();
        }
        result
    }
    pub fn get(&self, id: &str) -> Option<&AgentLaunch> {
        self.agents.get(id)
    }
    pub fn iter(&self) -> impl Iterator<Item = &AgentLaunch> {
        self.agents.values()
    }
}
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
