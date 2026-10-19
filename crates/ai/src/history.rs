//! Neutral document values, borrowed format projection and CPU-only admission rules.
use serde::{Deserialize, Serialize};
mod allocations;
mod budget;
mod catalog;
mod codec;
mod compatibility;
mod decode;
mod document;
pub mod native;
pub use native::NativeDocumentWire;
mod rows;
pub use rows::{
    DecodedDisplay, DecodedDisplayOutcome, DecodedRow, DecodedTranscript, SavedRowKind,
    ToolRowHeader, TranscriptBuildError,
};
mod encoding;
mod managed;
pub use managed::{decision_document, decision_managed};
mod error;
mod literal_preflight;
pub mod live;
mod loading;
mod preflight;
pub use literal_preflight::LiteralPreflight;
mod projection;
mod retained_json;
pub use retained_json::{HistoryJsonRetention, RetentionError, visit_chat_json_allocations};
pub mod dispatch;
mod size;
pub mod source;
mod wire;
pub mod writing;
pub use budget::{HistoryBudget, PreparedBudget, PromptBudget};
pub use catalog::{ChatSummary, ChatSummaryWire, FileStamp, decode_summary};
pub use codec::{
    decision_shared, decode, encode, encode_shared, prepare, prepare_imported, projection_shared,
    structural_bytes, verify_selected,
};
pub use compatibility::{
    CompatibilityReason, LegacySizing, LegacySizingError, PreparedHistory, PreparedProjection,
};
pub use decode::bounded as bounded_collection;
pub use document::{
    ArchivedDraftPatch, ChatMetadata, ChatWrite, ComposerData, ResumeDescriptor, SavedChat,
    SavedDocument, SavedPrompt, SharedChat, TranscriptData,
};
pub use encoding::{EncodingDecision, HistoryEncoding, HistoryOrigin, select_encoding};
pub use error::HistoryCodecError;
pub use loading::{
    HistoryGate, HistoryPermit, ManagedReaderFuture, NativeLoadError, NativeMaterialized,
    NativePhase, PreparedReaderLoad, ReaderCompletion, ReaderDelivery, ReaderDocumentId,
    ReaderFailure, ReaderInstallError, ReaderMaterializer, ReaderOutcome, ReaderPreparationFailure,
    ReaderRequest, ReaderSource, ReaderSourceError,
};
pub use preflight::{PreflightBounds, PreflightFailure, check_decoder as preflight_decoder};
pub use projection::{HistoryFraming, HistoryPrompts, HistoryShape, HistoryView};
pub use size::{FormatSize, MeasurementError, measure as measure_json, retained as retained_bytes};
pub const VERSION: u32 = 2;
pub const FORMAT: &str = "nocterm.chat";
pub const MAX_SAVED_IMAGE_BYTES: usize = 512 * 1024;
pub const MAX_SAVED_ENTRIES: usize = 2000;
pub const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_RESTORED: usize = 200;
pub const MAX_RESIDENT_BYTES: usize = preflight::LIMIT;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum SavedAttachment {
    Connection(String),
    Group(String),
    LocalTerminal(String),
}
fn check_memory(bytes: usize) -> Result<(), HistoryCodecError> {
    if bytes > MAX_RESIDENT_BYTES {
        Err(HistoryCodecError::StructuralLimit)
    } else {
        Ok(())
    }
}
fn check_size(bytes: usize) -> Result<(), HistoryCodecError> {
    if bytes > MAX_FILE_BYTES as usize {
        Err(HistoryCodecError::EncodedLimit)
    } else {
        Ok(())
    }
}
pub fn content_size(content: &crate::model::content::Content) -> Result<usize, HistoryCodecError> {
    size::encoded_len(content).map_err(HistoryCodecError::from)
}
pub fn prompt_size(prompt: &SavedPrompt) -> Result<usize, HistoryCodecError> {
    size::encoded_len(prompt).map_err(HistoryCodecError::from)
}
pub fn prompt_memory(prompt: &SavedPrompt) -> Result<usize, HistoryCodecError> {
    size::structural_size(prompt).map_err(HistoryCodecError::from)
}
#[cfg(test)]
mod draft_tests;

pub mod retention;

pub mod physical;
