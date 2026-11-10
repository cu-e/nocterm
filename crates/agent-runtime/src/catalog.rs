//! Installation-wide catalog payload reservations and immutable scoped snapshots.
use super::{AgentStorage, HistoryReader, Runtime};
use crate::history_admission::{HistoryAdmissionError, HistoryAdmissionHandle, HistoryBytesPermit};
use gpui::{Context, Task};
use nocterm_ai::{
    execution_origin::ChatKey,
    history::{ChatSummary, ContextId, HistoryGate, SavedChat},
    local_environment::ChatScope,
    storage::{AgentStorageError, ChatRepository, require_local_scope},
};
use std::sync::Arc;
mod allocation;
pub(crate) use allocation::CatalogAllocation;
#[cfg(test)]
pub(crate) use allocation::WORK_BYTES as CATALOG_WORK_BYTES;
mod prepared;
mod reclamation;
use prepared::PreparedRecords;

#[derive(Clone, Debug, thiserror::Error)]
pub enum CatalogLoadError {
    #[error("The original account source is unavailable: {0}")]
    AccountSource(#[from] nocterm_hubs_model::source_identity::LocalAccountSourceError),
    #[error("All normal installation history payload slots are retained or loading.")]
    Capacity,
    #[error(
        "Catalog metadata needs {incoming} bytes with {retained} retained; installation limit is {limit}."
    )]
    ResidentCapacity {
        retained: usize,
        incoming: usize,
        limit: usize,
    },
    #[error("Catalog metadata admission failed: {0}")]
    Admission(#[source] Arc<HistoryAdmissionError>),
    #[error(transparent)]
    Storage(#[from] AgentStorageError),
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
            require_local_scope(&ContextId::General.into(), &chat.header.chat_scope())?;
            if !ids.insert(chat.header.id.clone()) {
                return Err(nocterm_ai::history::HistoryCodecError::IdentityMismatch.into());
            }
            let chat = nocterm_ai::history::prepare_owned(chat)?;
            resident = resident.saturating_add(chat.resident_bytes());
            if resident > crate::history_admission::MAX_NORMAL_HISTORY_BYTES {
                return Err(AgentStorageError::ResidentCapacity {
                    retained: 0,
                    incoming: resident,
                    limit: crate::history_admission::MAX_NORMAL_HISTORY_BYTES,
                });
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
    scope: ChatScope,
    generation: u64,
    rows: Vec<ChatSummary>,
    error: Option<AgentStorageError>,
    allocation: CatalogAllocation,
}
impl CatalogSnapshot {
    pub fn scope(&self) -> ContextId {
        self.payload.scope.context()
    }
    pub fn chat_scope(&self) -> &ChatScope {
        &self.payload.scope
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
    pub fn chat_scope(&self) -> &ChatScope {
        self.snapshot.chat_scope()
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
    saved: PreparedRecords,
    scopes: std::collections::HashMap<ChatScope, ScopeCatalog>,
    gate: HistoryGate,
    repository: Arc<dyn ChatRepository>,
    closing: bool,
    admission: HistoryAdmissionHandle,
    sequence: u64,
    _reclamation: crate::history_admission::HistoryRegistration,
}
impl Catalog {
    pub(super) fn new(
        storage: &mut AgentStorage,
        admission: HistoryAdmissionHandle,
        prepared: Option<crate::history_admission::HistoryPayloadPermit>,
        cx: &mut Context<Runtime>,
    ) -> Self {
        let reclamation = admission
            .register(std::rc::Rc::new(reclamation::CatalogReclamation::new(
                cx.weak_entity(),
            )))
            .expect("a new installation admission is open");
        let mut catalog = Self {
            saved: PreparedRecords::new(
                storage.take_history().map(|history| history.0),
                &admission,
                prepared,
                cx.entity_id(),
            ),
            scopes: Default::default(),
            gate: HistoryGate::default(),
            repository: storage.chats(),
            closing: false,
            admission,
            sequence: 0,
            _reclamation: reclamation,
        };
        if catalog.saved.prepared_count().is_some() {
            if let Err(error) = catalog.install_prepared(cx) {
                tracing::warn!(%error, "could not admit prepared history metadata");
            }
        } else {
            let _ = catalog.reload(ContextId::General.into(), cx);
        }
        catalog
    }
    fn reserve(
        &mut self,
        scope: ChatScope,
        bytes: usize,
        cx: &mut Context<Runtime>,
    ) -> Result<(u64, CatalogAllocation), CatalogLoadError> {
        if self.closing {
            return Err(CatalogLoadError::Closed);
        }
        let generation = self
            .sequence
            .checked_add(1)
            .ok_or(CatalogLoadError::GenerationExhausted)?;
        let permit = match self
            .admission
            .reserve_payload_reclaiming(Some(cx.entity_id()), cx)
        {
            Ok(permit) => permit,
            Err(HistoryAdmissionError::Closed) => return Err(CatalogLoadError::Closed),
            Err(_) => {
                let victim = self
                    .reclaimable_scope(Some(&scope))
                    .ok_or(CatalogLoadError::Capacity)?;
                self.scopes.remove(&victim);
                self.admission
                    .reserve_payload()
                    .map_err(|error| match error {
                        HistoryAdmissionError::Closed => CatalogLoadError::Closed,
                        _ => CatalogLoadError::Capacity,
                    })?
            }
        };
        let bytes = self.reserve_metadata(&scope, bytes, cx)?;
        self.sequence = generation;
        Ok((generation, CatalogAllocation::new(permit, bytes)))
    }
    fn reserve_metadata(
        &mut self,
        scope: &ChatScope,
        bytes: usize,
        cx: &mut Context<Runtime>,
    ) -> Result<HistoryBytesPermit, CatalogLoadError> {
        loop {
            match self
                .admission
                .reserve_entity_bytes(cx.entity_id(), bytes, cx)
            {
                Ok(permit) => return Ok(permit),
                Err(error @ HistoryAdmissionError::ResidentCapacity { .. }) => {
                    // The borrowed Runtime was excluded from callbacks. Its
                    // own unpinned cached scopes can be reclaimed directly.
                    let Some(victim) = self.reclaimable_scope(Some(scope)) else {
                        return Err(catalog_admission_error(error));
                    };
                    self.scopes.remove(&victim);
                }
                Err(error) => return Err(catalog_admission_error(error)),
            }
        }
    }
    fn reclaimable_scope(&self, excluding: Option<&ChatScope>) -> Option<ChatScope> {
        self.scopes
            .iter()
            .filter(|(scope, entry)| excluding != Some(*scope) && entry.loading.is_none())
            .filter_map(|(scope, entry)| {
                entry
                    .snapshot
                    .as_ref()
                    .filter(|snapshot| Arc::strong_count(&snapshot.payload) == 1)
                    .map(|snapshot| (scope, snapshot.payload.allocation.order()))
            })
            .min_by_key(|(_, order)| *order)
            .map(|(scope, _)| scope.clone())
    }
    fn install_prepared(&mut self, cx: &mut Context<Runtime>) -> Result<(), CatalogLoadError> {
        let count = self.saved.prepared_count().unwrap_or(0);
        let bytes = (count + 1)
            .saturating_mul(allocation::MAX_ROW_BYTES)
            .saturating_add(1024 * 1024);
        let scope = ContextId::General.into();
        let (generation, allocation) = self.reserve(scope, bytes, cx)?;
        let rows = CatalogAllocation::check(Ok(self.saved.rows().unwrap_or_default()))?;
        let allocation = allocation
            .finish(&self.admission, &rows, rows.capacity())
            .map_err(catalog_admission_error)?;
        let payload = Arc::new(CatalogPayload {
            scope: ContextId::General.into(),
            generation,
            rows,
            error: None,
            allocation,
        });
        self.scopes.insert(
            ContextId::General.into(),
            ScopeCatalog {
                snapshot: Some(CatalogSnapshot { payload }),
                generation,
                loading: None,
            },
        );
        Ok(())
    }
    pub(super) fn ensure(
        &mut self,
        scope: ChatScope,
        cx: &mut Context<Runtime>,
    ) -> Result<(), CatalogLoadError> {
        if self.closing {
            return Err(CatalogLoadError::Closed);
        }
        if self.scopes.contains_key(&scope) {
            return Ok(());
        }
        if scope == ContextId::General.into() && self.saved.prepared_count().is_some() {
            return self.install_prepared(cx);
        }
        self.reload(scope, cx)
    }
    pub(super) fn reload(
        &mut self,
        scope: ChatScope,
        cx: &mut Context<Runtime>,
    ) -> Result<(), CatalogLoadError> {
        let (generation, allocation) = self.reserve(scope.clone(), allocation::WORK_BYTES, cx)?;
        let entry = self.scopes.entry(scope.clone()).or_default();
        entry.generation = generation;
        let repository = self.repository.clone();
        let captured = scope.clone();
        let (complete, loading) = futures::channel::oneshot::channel();
        cx.background_executor()
            .spawn(async move {
                let rows = CatalogAllocation::check(repository.catalog_in(captured).await);
                // The detached bounded read owns its allocation independently of a
                // cancelled viewer. An abandoned result drops bytes and proof together.
                let _ = complete.send((rows, allocation));
            })
            .detach();
        entry.loading = Some(cx.spawn(async move |runtime, cx| {
            let Ok((rows, allocation)) = loading.await else {
                return;
            };
            let _ = runtime.update(cx, |runtime, cx| {
                runtime.chats.loaded(scope, generation, rows, allocation);
                cx.notify();
            });
        }));
        Ok(())
    }
    pub(super) fn complete(
        &mut self,
        scope: ChatScope,
        generation: u64,
        rows: Result<Vec<ChatSummary>, AgentStorageError>,
        allocation: CatalogAllocation,
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
            Ok(mut rows) => {
                rows.retain(|row| {
                    require_local_scope(&scope, &row.chat_scope()).is_ok() && row.validate().is_ok()
                });
                rows.truncate(nocterm_ai::history::MAX_RESTORED);
                (rows, None)
            }
            Err(error) => {
                tracing::warn!(%error, "could not load agent catalog");
                (Vec::new(), Some(error))
            }
        };
        if scope == ContextId::General.into()
            && let Some(prepared) = self.saved.rows()
        {
            for row in prepared {
                rows.retain(|known| known.id != row.id);
                rows.push(row);
            }
            rows.sort_by_key(|row| std::cmp::Reverse((row.pinned, row.updated)));
            rows.truncate(nocterm_ai::history::MAX_RESTORED);
        }
        let allocation = match allocation.finish(&self.admission, &rows, rows.capacity()) {
            Ok(allocation) => allocation,
            Err(error) => {
                entry.loading.take();
                tracing::warn!(%error, "could not retain catalog metadata");
                return;
            }
        };
        let payload = Arc::new(CatalogPayload {
            scope,
            generation,
            rows,
            error,
            allocation,
        });
        entry.snapshot = Some(CatalogSnapshot { payload });
        entry.loading.take();
    }
    pub(super) fn invalidate(&mut self, scope: ChatScope) {
        self.scopes.remove(&scope);
    }
    pub(super) fn snapshot(&self, scope: ChatScope) -> Option<CatalogSnapshot> {
        self.scopes.get(&scope)?.snapshot.clone()
    }
    pub(super) fn oldest_payload(&self) -> Option<u64> {
        let scope = self.reclaimable_scope(None)?;
        Some(
            self.scopes
                .get(&scope)?
                .snapshot
                .as_ref()?
                .payload
                .allocation
                .order(),
        )
    }
    pub(super) fn evict_payload(&mut self) -> bool {
        let Some(scope) = self.reclaimable_scope(None) else {
            return false;
        };
        self.scopes.remove(&scope).is_some()
    }
    pub(super) fn take_prepared(
        &mut self,
        key: &ChatKey,
        payload: crate::history_admission::HistoryPayloadPermit,
        cx: &mut gpui::App,
    ) -> Result<Option<nocterm_core::allocation::Allocated<SavedChat>>, AgentStorageError> {
        self.saved.take(key, payload, cx)
    }
    pub(super) fn discard_prepared(&mut self, key: &ChatKey) {
        self.saved.discard(key);
    }

    pub(super) fn reader(&self, scope: ChatScope) -> HistoryReader {
        HistoryReader::new(self.repository.clone(), self.gate.clone(), scope)
    }
    pub(super) fn unavailable_reader(&self) -> HistoryReader {
        HistoryReader::unavailable(self.repository.clone(), self.gate.clone())
    }
    pub(super) fn stop(&mut self) {
        self.closing = true;
        for entry in self.scopes.values_mut() {
            entry.loading.take();
        }
    }
}

fn catalog_admission_error(error: HistoryAdmissionError) -> CatalogLoadError {
    match error {
        HistoryAdmissionError::Closed => CatalogLoadError::Closed,
        HistoryAdmissionError::PayloadCapacity => CatalogLoadError::Capacity,
        HistoryAdmissionError::ResidentCapacity {
            retained,
            incoming,
            limit,
        } => CatalogLoadError::ResidentCapacity {
            retained,
            incoming,
            limit,
        },
        HistoryAdmissionError::Storage(error) => CatalogLoadError::Storage(error),
        error => CatalogLoadError::Admission(Arc::new(error)),
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
            Err(AgentStorageError::ResidentCapacity {
                limit: crate::history_admission::MAX_NORMAL_HISTORY_BYTES,
                ..
            })
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
