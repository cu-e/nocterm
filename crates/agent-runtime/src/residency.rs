//! One installation admission over live histories and retained persistence values.
use crate::{Client, HistoryUsage};
use gpui::{App, EntityId};
use nocterm_ai::{
    execution_origin::ChatKey, history::MAX_RESIDENT_BYTES, storage::AgentStorageError,
};

pub(crate) struct HistoryResidency {
    clients: Vec<Client>,
    retained: usize,
}
impl HistoryResidency {
    pub(crate) fn new(clients: Vec<Client>, retained: usize) -> Self {
        Self { clients, retained }
    }
    pub(crate) fn admit(
        self,
        requester: Option<EntityId>,
        incoming: usize,
        cx: &mut App,
    ) -> Result<Vec<ChatKey>, AgentStorageError> {
        let mut observed: Vec<(Client, HistoryUsage)> = self
            .clients
            .into_iter()
            // The caller may already mutably borrow its entity. Never project it.
            .filter(|client| Some(client.id()) != requester && client.alive())
            .filter_map(|client| client.history_usage(cx).map(|usage| (client, usage)))
            .collect();
        let mut resident = observed.iter().fold(self.retained, |bytes, (_, usage)| {
            bytes.saturating_add(usage.bytes())
        });
        observed.sort_by_key(|(_, usage)| usage.updated());
        let mut evicted = Vec::new();
        for (client, usage) in observed {
            if resident.saturating_add(incoming) <= MAX_RESIDENT_BYTES {
                break;
            }
            if usage.shown() || usage.bytes() == 0 {
                continue;
            }
            let key = client.state(cx).map(|state| state.origin().key().clone());
            if client.evict_history(cx) {
                resident = resident.saturating_sub(usage.bytes());
                if let Some(key) = key {
                    evicted.push(key);
                }
            }
        }
        if resident.saturating_add(incoming) > MAX_RESIDENT_BYTES {
            return Err(AgentStorageError::ResidentCapacity {
                retained: resident,
                incoming,
                limit: MAX_RESIDENT_BYTES,
            });
        }
        Ok(evicted)
    }
}
