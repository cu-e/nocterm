//! Existing local session clients participate through their unchanged history gates.
use super::HistoryParticipant;
use crate::{Client, HistoryUsage};
use gpui::{App, EntityId};
use nocterm_ai::{execution_origin::ChatKey, storage::AgentStorageError};

pub(crate) struct SessionHistory(Client);
impl SessionHistory {
    pub(crate) fn new(client: Client) -> Self {
        Self(client)
    }
}
impl HistoryParticipant for SessionHistory {
    fn entity_id(&self) -> Option<EntityId> {
        Some(self.0.id())
    }
    fn usage(&self, cx: &App) -> Result<Option<HistoryUsage>, AgentStorageError> {
        Ok(self.0.alive().then(|| self.0.history_usage(cx)).flatten())
    }
    fn evict(&self, cx: &mut App) -> bool {
        self.0.evict_history(cx)
    }
    fn oldest_payload(&self, cx: &App) -> Option<u64> {
        self.0.history_payload(cx)
    }
    fn evict_payload(&self, cx: &mut App) -> bool {
        self.0.evict_history(cx)
    }
    fn local_key(&self, cx: &App) -> Option<ChatKey> {
        self.0.state(cx).map(|state| state.origin().key().clone())
    }
}
