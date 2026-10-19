//! Pure serialized document fields, without retention, delivery or cache state.
use super::{ChatMetadata, ComposerData, ResumeDescriptor, TranscriptData};
use serde::Serialize;

/// The original JSON document shape. Controlled intake supplies native field
/// owners before assembling this DTO; the DTO alone proves no allocation budget.
#[derive(Debug, Serialize)]
pub struct SavedDocument {
    pub id: String,
    pub metadata: ChatMetadata,
    pub resume: ResumeDescriptor,
    pub transcript: TranscriptData,
    pub composer: ComposerData,
}
impl SavedDocument {
    pub fn view(&self) -> Result<super::HistoryView<'_>, super::HistoryCodecError> {
        let skip = self
            .transcript
            .entries
            .len()
            .saturating_sub(crate::history::MAX_SAVED_ENTRIES);
        crate::history::HistoryView::from_parts(
            crate::history::HistoryFraming {
                id: &self.id,
                metadata: &self.metadata,
                resume: &self.resume,
                draft: self.composer.draft.as_deref(),
                attachments: &self.composer.attachments,
                times: self.transcript.times.get(skip..).unwrap_or_default(),
            },
            &self.transcript.entries[skip..],
            &self.composer.queue,
        )
    }
}
