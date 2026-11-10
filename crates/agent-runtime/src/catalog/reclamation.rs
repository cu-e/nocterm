//! Weak catalog reclamation participates in the installation's shared payload cap.
use crate::{HistoryUsage, Runtime, history_admission::HistoryParticipant};
use gpui::{App, EntityId, WeakEntity};
use nocterm_ai::storage::AgentStorageError;

pub(super) struct CatalogReclamation(WeakEntity<Runtime>);
impl CatalogReclamation {
    pub(super) fn new(runtime: WeakEntity<Runtime>) -> Self {
        Self(runtime)
    }
}
impl HistoryParticipant for CatalogReclamation {
    fn entity_id(&self) -> Option<EntityId> {
        Some(self.0.entity_id())
    }
    fn usage(&self, _cx: &App) -> Result<Option<HistoryUsage>, AgentStorageError> {
        Ok(None)
    }
    fn oldest_payload(&self, cx: &App) -> Option<u64> {
        self.0
            .upgrade()
            .and_then(|runtime| runtime.read(cx).chats.oldest_catalog_payload())
    }
    fn evict_payload(&self, cx: &mut App) -> bool {
        self.0.upgrade().is_some_and(|runtime| {
            runtime.update(cx, |runtime, cx| {
                let evicted = runtime.chats.evict_catalog_payload();
                if evicted {
                    cx.notify();
                }
                evicted
            })
        })
    }
}
