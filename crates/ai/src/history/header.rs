//! Durable identity and catalog metadata, independent of transcript payloads.
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
        }
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
}
impl IncomingHeader {
    pub(super) fn finish(self, version: u32) -> Result<SavedHeader, super::HistoryCodecError> {
        let scope = resolve_scope(version, self.scope)?;
        Ok(SavedHeader {
            id: self.id,
            agent_id: self.agent_id,
            title: self.title,
            name: self.name,
            pinned: self.pinned,
            updated: self.updated,
            scope,
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
        (VERSION, Some(scope)) => Ok(scope),
        (VERSION, None) => Err(HistoryCodecError::MissingScope),
        (version, _) => Err(HistoryCodecError::UnsupportedVersion(version)),
    }
}
