use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
#[derive(Clone, Default, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct AgentStateFile {
    pub favorites: BTreeMap<String, BTreeMap<String, BTreeSet<String>>>,
}
impl AgentStateFile {
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
