use super::{AgentCatalog, AgentLaunch, AgentProfile, valid_id};
use crate::{AgentServerSettings, AiSettings};
use std::collections::BTreeMap;

/// Where an available agent's launch comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentSource {
    /// A catalog profile, possibly overridden by settings.
    Builtin,
    /// A command the user added in settings.
    Custom,
}

/// An agent configuration that was skipped; the agent is not offered.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AgentConfigWarning {
    #[error("Invalid agent configuration: {0}")]
    Invalid(String),
    #[error("Missing or invalid command: {0}")]
    Command(String),
}

/// One agent the user can start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvailableAgent {
    launch: AgentLaunch,
    source: AgentSource,
    profile: Option<AgentProfile>,
}

impl AvailableAgent {
    pub fn id(&self) -> &str {
        &self.launch.id
    }
    pub fn name(&self) -> &str {
        &self.launch.name
    }
    pub fn launch(&self) -> &AgentLaunch {
        &self.launch
    }
    pub fn source(&self) -> AgentSource {
        self.source
    }
    /// The known agent this launch runs, if any.
    pub fn profile(&self) -> Option<&AgentProfile> {
        self.profile.as_ref()
    }
}

/// The agents that can be started under the current settings, by id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AvailableAgents {
    agents: BTreeMap<String, AvailableAgent>,
    warnings: Vec<AgentConfigWarning>,
}

impl AvailableAgents {
    /// Catalog agents, overridden, disabled or extended by `settings`.
    pub fn resolve(catalog: &AgentCatalog, settings: &AiSettings) -> Self {
        let mut launches: BTreeMap<String, (AgentLaunch, AgentSource)> = catalog
            .iter()
            .map(|profile| {
                let launch = profile.default_launch();
                (launch.id.clone(), (launch, AgentSource::Builtin))
            })
            .collect();
        let mut warnings = Vec::new();
        for (id, setting) in &settings.agents {
            if let Err(warning) = apply(&mut launches, id, setting) {
                warnings.push(warning);
            }
        }
        if !settings.enabled {
            launches.clear();
        }
        let agents = launches
            .into_iter()
            .map(|(id, (launch, source))| {
                let profile = catalog.for_launch(&launch).copied();
                let agent = AvailableAgent {
                    launch,
                    source,
                    profile,
                };
                (id, agent)
            })
            .collect();
        Self { agents, warnings }
    }

    pub fn get(&self, id: &str) -> Option<&AvailableAgent> {
        self.agents.get(id)
    }

    /// Agents in id order.
    pub fn iter(&self) -> impl Iterator<Item = &AvailableAgent> {
        self.agents.values()
    }

    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }

    /// Configurations that were skipped, in settings order.
    pub fn warnings(&self) -> &[AgentConfigWarning] {
        &self.warnings
    }
}

/// Applies one settings entry; a rejected entry also removes the agent.
fn apply(
    launches: &mut BTreeMap<String, (AgentLaunch, AgentSource)>,
    id: &str,
    setting: &AgentServerSettings,
) -> Result<(), AgentConfigWarning> {
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
        launches.remove(id);
        return Err(AgentConfigWarning::Invalid(id.to_owned()));
    }
    if !setting.enabled {
        launches.remove(id);
        return Ok(());
    }
    let (entry, _) = launches.entry(id.to_owned()).or_insert_with(|| {
        let launch = AgentLaunch {
            id: id.to_owned(),
            name: id.to_owned(),
            command: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
            inherit_env: Vec::new(),
        };
        (launch, AgentSource::Custom)
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
        launches.remove(id);
        return Err(AgentConfigWarning::Command(id.to_owned()));
    }
    Ok(())
}
