use crate::{
    acp,
    session_config::{self, Choices, ConfigValue, StoredOption},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
#[derive(Clone, Default, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct AgentStateFile {
    pub favorites: BTreeMap<String, BTreeMap<String, BTreeSet<String>>>,
    /// The agent the last new chat was started with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_agent: Option<String>,
    /// The configuration options each agent last reported.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, Vec<StoredOption>>,
    /// The model and reasoning effort last chosen for each agent.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub choices: BTreeMap<String, Choices>,
}
impl AgentStateFile {
    /// Keeps the options `agent` reported. Returns whether they changed.
    pub fn remember_options(&mut self, agent: &str, options: &[acp::SessionConfigOption]) -> bool {
        let stored = session_config::store(options);
        if stored.is_empty() || self.options.get(agent) == Some(&stored) {
            return false;
        }
        self.options.insert(agent.into(), stored);
        true
    }
    /// Keeps a choice of `option` for `agent`'s next chats, when it carries
    /// over. Returns whether it changed.
    pub fn remember_choice(
        &mut self,
        agent: &str,
        option: &acp::SessionConfigOption,
        value: &ConfigValue,
    ) -> bool {
        if !session_config::remembered(option) {
            return false;
        }
        self.choices
            .entry(agent.into())
            .or_default()
            .insert(option.id.to_string(), value.clone())
            .as_ref()
            != Some(value)
    }
    /// The choices `agent`'s new chats start with.
    pub fn choices(&self, agent: &str) -> Choices {
        self.choices.get(agent).cloned().unwrap_or_default()
    }
    /// The options a chat of `agent` shows before its session opens: the
    /// last reported ones with the remembered choices.
    pub fn preview(&self, agent: &str) -> Vec<acp::SessionConfigOption> {
        let mut options =
            session_config::restore(self.options.get(agent).map_or(&[], Vec::as_slice));
        for (id, value) in self.choices.get(agent).into_iter().flatten() {
            session_config::choose(&mut options, id, value);
        }
        options
    }
    pub fn contains(&self, agent: &str, option: &str, value: &str) -> bool {
        self.favorites
            .get(agent)
            .and_then(|v| v.get(option))
            .is_some_and(|v| v.contains(value))
    }
    pub fn toggle(&mut self, agent: &str, option: &str, value: &str) -> bool {
        let values = self
            .favorites
            .entry(agent.into())
            .or_default()
            .entry(option.into())
            .or_default();
        if values.remove(value) {
            false
        } else {
            values.insert(value.into());
            true
        }
    }
    pub fn sort<'a, T>(
        &self,
        agent: &str,
        option: &str,
        values: &mut [T],
        id: impl Fn(&T) -> &'a str,
    ) {
        values.sort_by_key(|v| !self.contains(agent, option, id(v)));
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(value) => toml::from_str(&value).map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.to_string()),
        }
    }
    /// Caller serializes writes in snapshot order. Atomic rename prevents partial files.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        nocterm_core::persist::save(path, self).map_err(|error| error.to_string())
    }
}
