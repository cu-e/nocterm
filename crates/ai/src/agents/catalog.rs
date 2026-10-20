use super::{AgentLaunch, AgentMark, AgentProfile, valid_id};
use std::sync::Arc;

/// A profile the catalog refuses, so a broken built-in fails at composition.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    #[error("agent profile id {0:?} is not a valid agent id")]
    InvalidId(&'static str),
    #[error("agent profile id {0:?} appears more than once")]
    DuplicateId(&'static str),
    #[error("agent profile {0:?} has no command")]
    MissingCommand(&'static str),
    #[error("agent profile {0:?} inherits an invalid or forbidden environment variable")]
    InvalidEnv(&'static str),
    #[error("agent profile {0:?} writes outside the home directory")]
    InvalidHomeState(&'static str),
}

/// The agents nocterm ships profiles for, in display order.
#[derive(Clone, Debug, Default)]
pub struct AgentCatalog(Arc<[AgentProfile]>);

impl AgentCatalog {
    pub fn new(profiles: impl IntoIterator<Item = AgentProfile>) -> Result<Self, CatalogError> {
        let profiles: Arc<[AgentProfile]> = profiles.into_iter().collect();
        for (index, profile) in profiles.iter().enumerate() {
            check(profile)?;
            if profiles[..index]
                .iter()
                .any(|seen| seen.id() == profile.id())
            {
                return Err(CatalogError::DuplicateId(profile.id()));
            }
        }
        Ok(Self(profiles))
    }

    pub fn get(&self, id: &str) -> Option<&AgentProfile> {
        self.0.iter().find(|profile| profile.id() == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &AgentProfile> {
        self.0.iter()
    }

    /// Whether `id` names a shipped agent whose launch settings may leave unset.
    pub fn is_builtin(&self, id: &str) -> bool {
        self.get(id).is_some()
    }

    /// The profile of the agent `launch` runs: the one with its id, otherwise
    /// the first whose marker its command mentions.
    pub fn for_launch(&self, launch: &AgentLaunch) -> Option<&AgentProfile> {
        self.get(&launch.id)
            .or_else(|| self.0.iter().find(|profile| profile.recognizes(launch)))
    }

    /// Every known agent's home state, relative to the home directory.
    pub fn home_state(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.0
            .iter()
            .flat_map(|profile| profile.home_state().iter().copied())
    }

    pub fn marks(&self) -> impl Iterator<Item = AgentMark> + '_ {
        self.0.iter().filter_map(AgentProfile::mark)
    }
}

fn check(profile: &AgentProfile) -> Result<(), CatalogError> {
    let id = profile.id();
    if !valid_id(id) {
        return Err(CatalogError::InvalidId(id));
    }
    let launch = profile.launch_defaults();
    if launch.command().trim().is_empty()
        || std::iter::once(launch.command())
            .chain(launch.args().iter().copied())
            .any(|part| part.contains('\0'))
    {
        return Err(CatalogError::MissingCommand(id));
    }
    if launch
        .inherit_env()
        .iter()
        .any(|name| !crate::env::valid_name(name) || crate::env::blocked(name))
    {
        return Err(CatalogError::InvalidEnv(id));
    }
    if profile.home_state().iter().any(|path| {
        let path = std::path::Path::new(path);
        path.as_os_str().is_empty()
            || !path
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)))
    }) {
        return Err(CatalogError::InvalidHomeState(id));
    }
    Ok(())
}
