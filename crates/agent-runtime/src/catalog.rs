//! Take-once history and bounded construction seeds, without filesystem access.
use super::{AgentStorage, HistoryReader, Runtime};
use gpui::{Context, Task};
use nocterm_ai::{
    history::{ChatSummary, HistoryGate, SavedChat},
    storage::{AgentStorageError, ChatRepository},
};
use std::sync::Arc;

/// Optional prepared history for hosts that already have bounded decoded records.
/// Production leaves this absent and loads the lightweight catalog lazily.
#[derive(Clone)]
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
            if !ids.insert(chat.id.clone()) {
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
pub(super) struct Catalog {
    saved: Option<Vec<SavedChat>>,
    rows: Option<Vec<ChatSummary>>,
    gate: HistoryGate,
    loading: Option<Task<()>>,
    repository: Arc<dyn ChatRepository>,
    closing: bool,
}
impl Catalog {
    pub(super) fn new(storage: &mut AgentStorage, cx: &mut Context<Runtime>) -> Self {
        let mut catalog = Self {
            saved: storage.take_history().map(|history| history.0),
            rows: None,
            gate: HistoryGate::default(),
            loading: None,
            repository: storage.chats(),
            closing: false,
        };
        if catalog.saved.is_none() {
            catalog.reload(cx)
        }
        catalog
    }
    pub(super) fn reload(&mut self, cx: &mut Context<Runtime>) {
        if self.closing {
            return;
        }
        let repository = self.repository.clone();
        let loading = cx
            .background_executor()
            .spawn(async move { repository.catalog().await });
        self.loading = Some(cx.spawn(async move |runtime, cx| {
            let rows = loading.await;
            let _ = runtime.update(cx, |runtime, cx| {
                runtime.chats.loaded(rows);
                cx.notify();
            });
        }));
    }
    pub(super) fn complete(&mut self, rows: Result<Vec<ChatSummary>, AgentStorageError>) {
        if self.closing {
            return;
        }
        self.rows = Some(rows.unwrap_or_else(|error| {
            tracing::warn!(%error,"could not load agent catalog");
            Vec::new()
        }));
        self.loading.take();
    }
    pub(super) fn take_saved(&mut self) -> Option<Vec<SavedChat>> {
        self.saved.take()
    }
    pub(super) fn take_rows(&mut self) -> Option<Vec<ChatSummary>> {
        self.rows.take()
    }
    pub(super) fn reader(&self) -> HistoryReader {
        HistoryReader::new(self.repository.clone(), self.gate.clone())
    }
    pub(super) fn stop(&mut self) {
        self.closing = true;
        self.loading.take();
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
