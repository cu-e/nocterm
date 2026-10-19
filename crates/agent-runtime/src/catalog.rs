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
            if !ids.insert(chat.id().to_owned()) {
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
    use nocterm_ai::{
        history::{ComposerData, HistoryCodecError, TranscriptData},
        model::{
            content::{Content, Media, MediaKind},
            retention::RetentionAuthority,
        },
        thread::Entry,
    };
    fn authority() -> RetentionAuthority {
        RetentionAuthority::new(usize::MAX).unwrap()
    }
    fn id(index: usize) -> String {
        format!("00000000-0000-0000-0000-{index:012x}")
    }
    /// A document built from public parts; `draft` and transcript are supplied.
    fn parts(
        index: usize,
        transcript: TranscriptData,
        draft: String,
    ) -> Result<SavedChat, HistoryCodecError> {
        let base = SavedChat::for_identity(&authority(), "codex".into(), id(index), 1);
        let (id, metadata, resume, _, _, _) = base.into_parts();
        SavedChat::from_parts(
            &authority(),
            id,
            metadata,
            resume,
            transcript,
            ComposerData {
                draft: Some(draft),
                ..Default::default()
            },
        )
    }
    fn chat(index: usize) -> SavedChat {
        parts(index, TranscriptData::default(), "draft".into()).unwrap()
    }
    /// Construction and preparation are one public path; either may refuse.
    fn prepare_one(
        built: Result<SavedChat, HistoryCodecError>,
    ) -> Result<PreparedHistory, AgentStorageError> {
        PreparedHistory::new(vec![built?])
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
        // JSON escapes each NUL as six bytes. Exceed the encoded limit without
        // first exceeding the independent resident-memory limit.
        let draft = "\0".repeat(nocterm_ai::history::MAX_FILE_BYTES as usize / 6 + 1);
        let result = prepare_one(parts(1, TranscriptData::default(), draft));
        assert!(
            matches!(
                result,
                Err(AgentStorageError::Codec(HistoryCodecError::EncodedLimit))
            ),
            "{:?}",
            result.err()
        );
    }
    #[test]
    fn prepared_records_reject_aggregate_structural_memory_without_trimming() {
        let chats = (1..=33)
            .map(|index| parts(index, TranscriptData::default(), "x".repeat(1024 * 1024)).unwrap())
            .collect();
        assert!(matches!(
            PreparedHistory::new(chats),
            Err(AgentStorageError::Codec(HistoryCodecError::StructuralLimit))
        ));
    }
    #[test]
    fn prepared_records_keep_live_rows_and_encode_only_the_saved_window() {
        let entries = (0..2001)
            .map(|i| Entry::Agent(format!("row-{i}")))
            .collect();
        let transcript = TranscriptData {
            entries,
            times: (0..2001).collect(),
        };
        let prepared = prepare_one(parts(1, transcript, "draft".into())).unwrap();
        let record = &prepared.records()[0];
        assert_eq!(record.transcript().entries.len(), 2001);
        assert_eq!(record.transcript().times.len(), 2001);
        let view = record.view().unwrap();
        assert_eq!(view.entries().len(), nocterm_ai::history::MAX_SAVED_ENTRIES);
        assert_eq!(view.entries().first(), Some(&Entry::Agent("row-1".into())));
        assert_eq!(view.framing().times.first(), Some(&1));
        let bytes = nocterm_ai::history::encode(record).unwrap();
        let restored =
            nocterm_ai::history::decode(record.retention(), &bytes, record.id()).unwrap();
        assert_eq!(restored.transcript().entries, view.entries());
        assert_eq!(restored.transcript().times, view.framing().times);
    }
    #[test]
    fn prepared_records_reject_excess_times_without_matching_rows() {
        let oversized = TranscriptData {
            entries: Vec::new(),
            times: vec![1; 2001],
        };
        let result = prepare_one(parts(2, oversized, "draft".into()));
        assert!(
            matches!(
                result,
                Err(AgentStorageError::Codec(HistoryCodecError::CollectionLimit))
            ),
            "{:?}",
            result.err()
        );
    }
    #[test]
    fn prepared_records_keep_large_image_contents_and_prepare_memory_budgets() {
        let image = Content::Media(Media {
            kind: MediaKind::Image,
            encoded: "A".repeat(600 * 1024),
            mime: "image/png".into(),
            uri: None,
            annotations: None,
            extensions: None,
        });
        let transcript = TranscriptData {
            entries: vec![Entry::Content(image)],
            times: Vec::new(),
        };
        let expected = serde_json::to_value(&transcript.entries).unwrap();
        let chat = parts(1, transcript, "draft".into()).unwrap();
        let prepared = PreparedHistory::new(vec![chat]).unwrap();
        let record = &prepared.records()[0];
        assert_eq!(
            serde_json::to_value(&record.transcript().entries).unwrap(),
            expected
        );
        assert!(record.resident_bytes() > 600 * 1024);
        assert!(record.prepared().is_some());
    }
}
