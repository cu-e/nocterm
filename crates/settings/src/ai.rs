//! The `[ai]` table: the master switch and the agents behind the AI panel.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
pub(crate) mod lifecycle;
use lifecycle::{AgentResourceSettings, AgentSessionSettings};

/// AI agents that work inside nocterm. Nothing runs until a thread is started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct AiSettings {
    /// Master switch. Off: the AI panel is hidden and every agent is stopped.
    pub enabled: bool,
    /// Whether agent processes run isolated (Linux, bubblewrap). Changing it
    /// restarts running agents.
    pub sandbox: SandboxMode,
    /// Agent id that new threads start with. Unset: ask each time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_agent: Option<String>,
    /// Folder agents start in. Unset: a private folder in the application's state directory.
    /// Without isolation, agents can read and change any of your files.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    /// Confirmation policies for the agent's own tools and attached terminals.
    pub approval: ApprovalSettings,
    /// Global limits for live agent sessions across all windows.
    pub sessions: AgentSessionSettings,
    /// Per-connection Linux process container limits.
    pub resources: AgentResourceSettings,
    /// Agents by id. An entry named like a built-in agent (`claude`, `codex`,
    /// `hermes`) changes it; any other id adds a custom agent.
    pub agents: BTreeMap<String, AgentServerSettings>,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            sandbox: SandboxMode::default(),
            default_agent: None,
            working_directory: None,
            approval: ApprovalSettings::default(),
            sessions: AgentSessionSettings::default(),
            resources: AgentResourceSettings::default(),
            agents: BTreeMap::new(),
        }
    }
}

/// How agent processes are isolated from the rest of the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    /// Agents run directly as the current user.
    #[default]
    Off,
    /// Agents may change only their working directory and their own state and
    /// caches; SSH/GPG keys, cloud credentials, password stores and nocterm's
    /// own files are hidden. Linux only, with bubblewrap installed; agents fail
    /// to start rather than run unisolated.
    Workspace,
}

/// Which agent actions need your confirmation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ApprovalSettings {
    /// Permission requests from the agent for its own files and tools. Allow
    /// selects a one-time approval only; providers without one still ask.
    pub agent_permissions: ApprovalPolicy,
    /// Reading the output of an attached terminal.
    pub terminal_read: ApprovalPolicy,
    /// Typing into an attached terminal, running a command in it, or connecting to an
    /// attached server in the background.
    pub terminal_write: ApprovalPolicy,
    /// Hide private keys, access tokens, passwords and credentials in URLs in terminal
    /// output before an agent reads it. This is best effort; do not attach terminals that
    /// show secrets.
    pub redact_secrets: bool,
}

impl Default for ApprovalSettings {
    fn default() -> Self {
        Self {
            agent_permissions: ApprovalPolicy::Ask,
            terminal_read: ApprovalPolicy::Allow,
            terminal_write: ApprovalPolicy::Ask,
            redact_secrets: true,
        }
    }
}

/// Whether an action runs straight away or waits for the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    Allow,
    Ask,
}

/// How to start one agent that speaks the Agent Client Protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct AgentServerSettings {
    /// Offer this agent in the panel.
    #[serde(skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// Name shown in the panel. Unset: the built-in name, or the id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Program to run. Unset: the built-in command. Required for custom agents.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Individual executable arguments, without shell parsing. Unset: the built-in arguments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    /// Additional environment variables for the agent; never put passwords here.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Names of your own environment variables the agent may inherit, such as an API key.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inherit_env: Vec<String>,
}

impl Default for AgentServerSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            name: None,
            command: None,
            args: None,
            env: BTreeMap::new(),
            inherit_env: Vec::new(),
        }
    }
}

fn is_true(value: &bool) -> bool {
    *value
}

impl AiSettings {
    /// Trims text values; an empty one counts as unset. Agent entries are kept
    /// as written: the registry skips the ones it cannot start, and the
    /// settings page explains why.
    pub(crate) fn sanitize(&mut self) {
        self.sessions.sanitize();
        self.resources.sanitize();
        trim_unset(&mut self.default_agent);
        trim_unset(&mut self.working_directory);
        for agent in self.agents.values_mut() {
            trim_unset(&mut agent.name);
            trim_unset(&mut agent.command);
        }
    }
}

fn trim_unset(value: &mut Option<String>) {
    *value = value
        .take()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty());
}

#[cfg(test)]
mod tests {
    use crate::Settings;

    use super::*;

    #[test]
    fn defaults_enable_ai_and_ask_before_writing() {
        let ai = AiSettings::default();

        assert!(ai.enabled);
        assert_eq!(ai.default_agent, None);
        assert_eq!(ai.approval.agent_permissions, ApprovalPolicy::Ask);
        assert_eq!(ai.approval.terminal_read, ApprovalPolicy::Allow);
        assert_eq!(ai.approval.terminal_write, ApprovalPolicy::Ask);
        assert!(ai.approval.redact_secrets);
        assert!(ai.agents.is_empty());
        assert_eq!(Settings::default().ai, ai);
    }

    #[test]
    fn a_partial_table_keeps_the_other_defaults() {
        let settings: Settings =
            toml::from_str("[ai]\nenabled = false\n[ai.approval]\nterminal_write = \"allow\"\n")
                .unwrap();

        assert!(!settings.ai.enabled);
        assert_eq!(settings.ai.approval.terminal_write, ApprovalPolicy::Allow);
        assert_eq!(settings.ai.approval.terminal_read, ApprovalPolicy::Allow);
        assert_eq!(settings.ai.approval.agent_permissions, ApprovalPolicy::Ask);
        assert!(settings.ai.approval.redact_secrets);
    }

    #[test]
    fn permission_policies_round_trip_independently() {
        for agent_permissions in [ApprovalPolicy::Ask, ApprovalPolicy::Allow] {
            for terminal_read in [ApprovalPolicy::Ask, ApprovalPolicy::Allow] {
                for terminal_write in [ApprovalPolicy::Ask, ApprovalPolicy::Allow] {
                    let mut settings = Settings::default();
                    settings.ai.approval.agent_permissions = agent_permissions;
                    settings.ai.approval.terminal_read = terminal_read;
                    settings.ai.approval.terminal_write = terminal_write;
                    let encoded = toml::to_string(&settings).unwrap();
                    assert_eq!(
                        toml::from_str::<Settings>(&encoded).unwrap().ai.approval,
                        settings.ai.approval
                    );
                }
            }
        }
    }

    #[test]
    fn unknown_keys_are_rejected_by_name() {
        let error = toml::from_str::<Settings>("[ai]\nenabeld = true\n").unwrap_err();
        assert!(error.to_string().contains("enabeld"), "{error}");
        let error =
            toml::from_str::<Settings>("[ai.agents.mine]\ncomand = \"mine\"\n").unwrap_err();
        assert!(error.to_string().contains("comand"), "{error}");
    }

    #[test]
    fn agents_are_read_by_id() {
        let settings: Settings = toml::from_str(
            "[ai.agents.mine]\ncommand = \"/opt/mine\"\nargs = [\"acp\"]\ninherit_env = [\"MINE_KEY\"]\n[ai.agents.mine.env]\nMODE = \"fast\"\n[ai.agents.hermes]\nenabled = false\n",
        )
        .unwrap();

        let mine = &settings.ai.agents["mine"];
        assert!(mine.enabled);
        assert_eq!(mine.command.as_deref(), Some("/opt/mine"));
        assert_eq!(mine.args.as_deref(), Some(&["acp".to_owned()][..]));
        assert_eq!(mine.inherit_env, ["MINE_KEY"]);
        assert_eq!(mine.env["MODE"], "fast");
        assert!(!settings.ai.agents["hermes"].enabled);
    }

    #[test]
    fn sanitizing_treats_blank_text_as_unset() {
        let mut settings = Settings::default();
        settings.ai.default_agent = Some("  ".to_owned());
        settings.ai.working_directory = Some(" /work ".to_owned());
        settings.ai.agents.insert(
            "mine".to_owned(),
            AgentServerSettings {
                name: Some(" ".to_owned()),
                command: Some(" mine ".to_owned()),
                ..AgentServerSettings::default()
            },
        );

        let settings = settings.sanitized();

        assert_eq!(settings.ai.default_agent, None);
        assert_eq!(settings.ai.working_directory.as_deref(), Some("/work"));
        assert_eq!(settings.ai.agents["mine"].name, None);
        assert_eq!(settings.ai.agents["mine"].command.as_deref(), Some("mine"));
    }
}
