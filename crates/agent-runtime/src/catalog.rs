//! Installation-wide catalog payload reservations and immutable scoped snapshots.
use super::{AgentStorage, HistoryReader, Runtime};
use gpui::{Context, Task};
use nocterm_ai::{
    execution_origin::ChatKey,
    history::{ChatSummary, ContextId, HistoryGate, SavedChat},
    storage::{AgentStorageError, ChatRepository, require_chat_scope},
};
use std::sync::{Arc, Weak};
const MAX_CATALOG_SNAPSHOTS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CatalogLoadError {
    #[error("All 32 installation catalog payload slots are retained or loading.")]
    Capacity,
    #[error("Agent history is shutting down.")]
    Closed,
    #[error("Agent history load generation is exhausted.")]
    GenerationExhausted,
}

/// Optional prepared history for hosts that already have bounded decoded records.
/// Production leaves this absent and loads the lightweight catalog lazily.
pub struct PreparedHistory(Vec<SavedChat>);
impl PreparedHistory {
    pub fn new(chats: Vec<SavedChat>) -> Result<Self, AgentStorageError> {
        if chats.len() > nocterm_ai::history::MAX_RESTORED {
            return Err(nocterm_ai::history::HistoryCodecError::CollectionLimit.into());
        }
        let mut resident = 0usize;
        let mut ids = std::collections::HashSet::new();
        let mut prepared = Vec::with_capacity(chats.len());
        for chat in chats {
            require_chat_scope(ContextId::General, chat.header.scope)?;
            if !ids.insert(chat.header.id.clone()) {
                return Err(nocterm_ai::history::HistoryCodecError::IdentityMismatch.into());
            }
            let chat = nocterm_ai::history::prepare(chat)?;
            resident = resident.saturating_add(chat.resident_bytes());
            if resident > nocterm_ai::history::MAX_RESIDENT_BYTES {
                return Err(nocterm_ai::history::HistoryCodecError::StructuralLimit.into());
            }
            prepared.push(chat);
        }
        Ok(Self(prepared))
    }
    pub fn records(&self) -> &[SavedChat] {
        &self.0
    }
}
/// Immutable light metadata for one captured context and load generation.
#[derive(Clone)]
pub struct CatalogSnapshot {
    payload: Arc<CatalogPayload>,
}
struct CatalogPayload {
    scope: ContextId,
    generation: u64,
    rows: Vec<ChatSummary>,
    error: Option<AgentStorageError>,
}
impl CatalogSnapshot {
    pub fn scope(&self) -> ContextId {
        self.payload.scope
    }
    pub fn generation(&self) -> u64 {
        self.payload.generation
    }
    pub fn rows(&self) -> &[ChatSummary] {
        &self.payload.rows
    }
    pub fn error(&self) -> Option<&AgentStorageError> {
        self.payload.error.as_ref()
    }
    pub fn row(&self, index: usize) -> Option<CatalogRow> {
        (index < self.rows().len()).then(|| CatalogRow {
            snapshot: self.clone(),
            index,
        })
    }
}
/// A row retains its original accounted payload; no metadata strings are copied.
#[derive(Clone)]
pub struct CatalogRow {
    snapshot: CatalogSnapshot,
    index: usize,
}
impl CatalogRow {
    pub fn scope(&self) -> ContextId {
        self.snapshot.scope()
    }
    pub fn generation(&self) -> u64 {
        self.snapshot.generation()
    }
    pub fn summary(&self) -> &ChatSummary {
        // Only CatalogSnapshot::row constructs this immutable validated index.
        &self.snapshot.rows()[self.index]
    }
}
#[derive(Default)]
struct ScopeCatalog {
    snapshot: Option<CatalogSnapshot>,
    generation: u64,
    loading: Option<Task<()>>,
}
pub(super) struct Catalog {
    saved: Option<Vec<SavedChat>>,
    scopes: std::collections::HashMap<ContextId, ScopeCatalog>,
    gate: HistoryGate,
    repository: Arc<dyn ChatRepository>,
    closing: bool,
    payloads: Vec<Weak<CatalogPayload>>,
    sequence: u64,
}
impl Catalog {
    pub(super) fn new(storage: &mut AgentStorage, cx: &mut Context<Runtime>) -> Self {
        let mut catalog = Self {
            saved: storage.take_history().map(|history| history.0),
            scopes: Default::default(),
            gate: HistoryGate::default(),
            repository: storage.chats(),
            closing: false,
            payloads: Vec::new(),
            sequence: 0,
        };
        if let Some(chats) = &catalog.saved {
            let payload = Arc::new(CatalogPayload {
                scope: ContextId::General,
                generation: 1,
                rows: chats.iter().map(ChatSummary::from_chat).collect(),
                error: None,
            });
            catalog.payloads.push(Arc::downgrade(&payload));
            catalog.sequence = 1;
            catalog.scopes.insert(
                ContextId::General,
                ScopeCatalog {
                    snapshot: Some(CatalogSnapshot { payload }),
                    generation: 1,
                    loading: None,
                },
            );
        } else {
            let _ = catalog.reload(ContextId::General, cx);
        }
        catalog
    }
    fn reserve(&mut self, scope: ContextId) -> Result<u64, CatalogLoadError> {
        if self.closing {
            return Err(CatalogLoadError::Closed);
        }
        let generation = self
            .sequence
            .checked_add(1)
            .ok_or(CatalogLoadError::GenerationExhausted)?;
        self.payloads.retain(|payload| payload.strong_count() > 0);
        let pending = self
            .scopes
            .iter()
            .filter(|(key, entry)| **key != scope && entry.loading.is_some())
            .count();
        if self.payloads.len() + pending >= MAX_CATALOG_SNAPSHOTS {
            let victim = self
                .scopes
                .iter()
                .filter(|(key, entry)| {
                    **key != scope
                        && entry.loading.is_none()
                        && entry
                            .snapshot
                            .as_ref()
                            .is_some_and(|snapshot| Arc::strong_count(&snapshot.payload) == 1)
                })
                .min_by_key(|(_, entry)| entry.generation)
                .map(|(key, _)| *key);
            let Some(victim) = victim else {
                return Err(CatalogLoadError::Capacity);
            };
            self.scopes.remove(&victim);
            self.payloads.retain(|payload| payload.strong_count() > 0);
        }
        self.sequence = generation;
        Ok(generation)
    }
    pub(super) fn ensure(
        &mut self,
        scope: ContextId,
        cx: &mut Context<Runtime>,
    ) -> Result<(), CatalogLoadError> {
        if self.closing {
            return Err(CatalogLoadError::Closed);
        }
        if self.scopes.contains_key(&scope) {
            return Ok(());
        }
        self.reload(scope, cx)
    }
    pub(super) fn reload(
        &mut self,
        scope: ContextId,
        cx: &mut Context<Runtime>,
    ) -> Result<(), CatalogLoadError> {
        let generation = self.reserve(scope)?;
        let entry = self.scopes.entry(scope).or_default();
        entry.generation = generation;
        let repository = self.repository.clone();
        let loading = cx
            .background_executor()
            .spawn(async move { repository.catalog(scope).await });
        entry.loading = Some(cx.spawn(async move |runtime, cx| {
            let rows = loading.await;
            let _ = runtime.update(cx, |runtime, cx| {
                runtime.chats.loaded(scope, generation, rows);
                cx.notify();
            });
        }));
        Ok(())
    }
    pub(super) fn complete(
        &mut self,
        scope: ContextId,
        generation: u64,
        rows: Result<Vec<ChatSummary>, AgentStorageError>,
    ) {
        if self.closing {
            return;
        }
        let Some(entry) = self.scopes.get_mut(&scope) else {
            return;
        };
        if entry.generation != generation {
            return;
        }
        let (mut rows, error): (Vec<ChatSummary>, _) = match rows {
            Ok(rows) => (
                rows.into_iter()
                    .filter(|row| {
                        require_chat_scope(scope, row.scope).is_ok() && row.validate().is_ok()
                    })
                    .take(nocterm_ai::history::MAX_RESTORED)
                    .collect(),
                None,
            ),
            Err(error) => {
                tracing::warn!(%error, "could not load agent catalog");
                (Vec::new(), Some(error))
            }
        };
        if scope == ContextId::General
            && let Some(prepared) = &self.saved
        {
            for chat in prepared {
                rows.retain(|row| row.id != chat.header.id);
                rows.push(ChatSummary::from_chat(chat));
            }
            rows.sort_by_key(|row| std::cmp::Reverse((row.pinned, row.updated)));
            rows.truncate(nocterm_ai::history::MAX_RESTORED);
        }
        let payload = Arc::new(CatalogPayload {
            scope,
            generation,
            rows,
            error,
        });
        self.payloads.retain(|payload| payload.strong_count() > 0);
        self.payloads.push(Arc::downgrade(&payload));
        entry.snapshot = Some(CatalogSnapshot { payload });
        entry.loading.take();
    }
    pub(super) fn invalidate(&mut self, scope: ContextId) {
        self.scopes.remove(&scope);
    }
    pub(super) fn snapshot(&self, scope: ContextId) -> Option<CatalogSnapshot> {
        self.scopes.get(&scope)?.snapshot.clone()
    }
    pub(super) fn take_prepared(&mut self, key: &ChatKey) -> Option<SavedChat> {
        if key.scope() != ContextId::General {
            return None;
        }
        let records = self.saved.as_mut()?;
        let index = records.iter().position(|chat| chat.header.id == key.id())?;
        Some(records.remove(index))
    }
    pub(super) fn prepared_bytes(&self) -> usize {
        self.saved
            .as_ref()
            .map_or(0, |saved| saved.iter().map(SavedChat::resident_bytes).sum())
    }
    pub(super) fn reader(&self, scope: ContextId) -> HistoryReader {
        HistoryReader::new(self.repository.clone(), self.gate.clone(), scope)
    }
    pub(super) fn stop(&mut self) {
        self.closing = true;
        for entry in self.scopes.values_mut() {
            entry.loading.take();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_ai::history::HistoryCodecError;
    fn chat(index: usize) -> SavedChat {
        let text = format!(
            r#"{{"version":1,"id":"00000000-0000-0000-0000-{index:012x}","agent_id":"codex","updated":1,"draft":"draft","entries":[]}}"#
        );
        nocterm_ai::history::decode(
            text.as_bytes(),
            &format!("00000000-0000-0000-0000-{index:012x}"),
        )
        .unwrap()
    }
    #[test]
    fn prepared_records_reject_excess_count_and_duplicate_identity() {
        assert!(matches!(
            PreparedHistory::new(vec![chat(1); 201]),
            Err(AgentStorageError::Codec(HistoryCodecError::CollectionLimit))
        ));
        assert!(matches!(
            PreparedHistory::new(vec![chat(1), chat(1)]),
            Err(AgentStorageError::Codec(
                HistoryCodecError::IdentityMismatch
            ))
        ));
    }
    #[test]
    fn prepared_records_reject_the_individual_file_limit() {
        let mut chat = chat(1);
        chat.draft = Some("x".repeat(nocterm_ai::history::MAX_FILE_BYTES as usize));
        assert!(matches!(
            PreparedHistory::new(vec![chat]),
            Err(AgentStorageError::Codec(HistoryCodecError::EncodedLimit))
        ));
    }
    #[test]
    fn prepared_records_reject_aggregate_structural_memory_without_trimming() {
        let chats = (1..=33)
            .map(|index| {
                let mut chat = chat(index);
                chat.draft = Some("x".repeat(1024 * 1024));
                chat
            })
            .collect();
        assert!(matches!(
            PreparedHistory::new(chats),
            Err(AgentStorageError::Codec(HistoryCodecError::StructuralLimit))
        ));
    }
    #[test]
    fn prepared_records_reject_unbounded_rows_without_serialization_trimming() {
        let mut oversized = chat(1);
        oversized.entries = (0..2001)
            .map(|_| nocterm_ai::thread::Entry::Agent("row".into()))
            .collect();
        assert!(matches!(
            PreparedHistory::new(vec![oversized]),
            Err(AgentStorageError::Codec(HistoryCodecError::CollectionLimit))
        ));
        let mut oversized = chat(2);
        oversized.times = vec![1; 2001];
        assert!(matches!(
            PreparedHistory::new(vec![oversized]),
            Err(AgentStorageError::Codec(HistoryCodecError::CollectionLimit))
        ));
    }
    #[test]
    fn prepared_records_keep_large_image_contents_and_prepare_memory_budgets() {
        let id = "00000000-0000-0000-0000-000000000001";
        let data = "A".repeat(600 * 1024);
        let json = serde_json::json!({
            "version": 1, "id": id, "agent_id": "codex", "updated": 1,
            "entries": [{"kind": "content", "value": {
                "type": "image", "data": data, "mimeType": "image/png"
            }}]
        });
        let chat = nocterm_ai::history::decode(&serde_json::to_vec(&json).unwrap(), id).unwrap();
        let prepared = PreparedHistory::new(vec![chat]).unwrap();
        let actual = serde_json::to_value(&prepared.records()[0]).unwrap();
        assert_eq!(actual["entries"], json["entries"]);
        assert!(prepared.records()[0].resident_bytes() > 600 * 1024);
        assert!(prepared.records()[0].prepared_budget().is_some());
    }
}
