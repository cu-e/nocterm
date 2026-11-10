//! Durable identity and catalog metadata, independent of transcript payloads.
use crate::local_environment::{ChatScope, LocalEnvironment};
use nocterm_project_model::ContextId;
use serde::{Deserialize, ser::SerializeMap};

/// Plain saved metadata. Membership does not select or launch an agent process.
#[derive(Clone, Debug)]
pub struct SavedHeader {
    /// Names the saved document; a supplied UUID.
    pub id: String,
    /// The registry identity of the agent the chat was held with.
    pub agent_id: String,
    /// The title supplied by the agent.
    pub title: Option<String>,
    /// The user's name, overriding the agent title.
    pub name: Option<String>,
    /// Pinned chats sort before unpinned history.
    pub pinned: bool,
    /// Seconds since the Unix epoch, supplied by the host.
    pub updated: u64,
    /// Durable chat membership, independent of names, paths and current selection.
    pub scope: ContextId,
    pub environment: LocalEnvironment,
}
impl SavedHeader {
    pub fn new(agent_id: String, id: String, updated: u64, scope: ContextId) -> Self {
        Self {
            id,
            agent_id,
            title: None,
            name: None,
            pinned: false,
            updated,
            scope,
            environment: LocalEnvironment::ThisComputer,
        }
    }
    pub fn chat_scope(&self) -> ChatScope {
        ChatScope::new(self.environment.clone(), self.scope)
    }
    pub(super) fn serialize_prefix<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        map.serialize_entry("id", &self.id)?;
        map.serialize_entry("agent_id", &self.agent_id)?;
        if let Some(title) = &self.title {
            map.serialize_entry("title", title)?;
        }
        if let Some(name) = &self.name {
            map.serialize_entry("name", name)?;
        }
        if self.pinned {
            map.serialize_entry("pinned", &self.pinned)?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
pub(super) struct IncomingHeader {
    id: String,
    agent_id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    pinned: bool,
    updated: u64,
    #[serde(default)]
    scope: Option<ContextId>,
    #[serde(default)]
    environment: Option<LocalEnvironment>,
}
impl IncomingHeader {
    pub(super) fn finish(self, version: u32) -> Result<SavedHeader, super::HistoryCodecError> {
        let scope = resolve_scope(version, self.scope)?;
        let environment = self
            .environment
            .ok_or(super::HistoryCodecError::MissingEnvironment)
            .or_else(|error| {
                if version <= 2 {
                    Ok(LocalEnvironment::ThisComputer)
                } else {
                    Err(error)
                }
            })?;
        validate_environment(version, &environment)?;
        Ok(SavedHeader {
            id: self.id,
            agent_id: self.agent_id,
            title: self.title,
            name: self.name,
            pinned: self.pinned,
            updated: self.updated,
            scope,
            environment,
        })
    }
}
pub(super) fn resolve_scope(
    version: u32,
    scope: Option<ContextId>,
) -> Result<ContextId, super::HistoryCodecError> {
    use super::{HistoryCodecError, LEGACY_VERSION, VERSION};
    match (version, scope) {
        (LEGACY_VERSION, None | Some(ContextId::General)) => Ok(ContextId::General),
        (LEGACY_VERSION, Some(ContextId::Project(_))) => Err(HistoryCodecError::LegacyScope),
        (VERSION | super::ENVIRONMENT_VERSION, Some(scope)) => Ok(scope),
        (VERSION | super::ENVIRONMENT_VERSION, None) => Err(HistoryCodecError::MissingScope),
        (version, _) => Err(HistoryCodecError::UnsupportedVersion(version)),
    }
}

pub(super) fn validate_environment(
    version: u32,
    environment: &LocalEnvironment,
) -> Result<(), super::HistoryCodecError> {
    match version {
        1 | 2 if !environment.is_this_computer() => {
            Err(super::HistoryCodecError::LegacyEnvironment)
        }
        1..=3 => Ok(()),
        other => Err(super::HistoryCodecError::UnsupportedVersion(other)),
    }
}
