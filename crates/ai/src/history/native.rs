//! Native construction crosses final history admission with its phase credit alive.
//! This owner does not parse wire grammar, choose file paths or create identities.
//! Inherent methods are generic capability operations only; history payload
//! construction and transitions are free functions in the child modules.
pub mod archive;
pub mod document;
mod wire;
pub use wire::NativeDocumentWire;
pub mod entries;
use super::{HistoryCodecError, HistoryOrigin, SavedChat};
use crate::model::{
    json::deferred::{JsonDecodeError, ledger::Reservation},
    native::NativeOwned,
};

#[must_use = "complete admission before releasing native construction credit"]
pub struct NativeChat {
    chat: SavedChat,
    allocation: Reservation,
}
impl NativeChat {
    pub(crate) fn belongs_to(&self, ledger: &crate::model::json::deferred::DecodeLedger) -> bool {
        self.allocation.belongs_to(ledger)
    }
    /// Test repository fixtures are already owned before action admission. This
    /// named fixture boundary is not the production parser's preallocation proof.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn from_fixture(
        chat: SavedChat,
        ledger: &crate::model::json::deferred::DecodeLedger,
    ) -> Result<Self, HistoryCodecError> {
        if !chat.retention().same(ledger.authority()) {
            return Err(HistoryCodecError::from(JsonDecodeError::Retention(
                crate::model::retention::RetentionError::ForeignAuthority,
            )));
        }
        let bytes = super::allocations::chat(&chat)?;
        let allocation = ledger.reserve(bytes).map_err(HistoryCodecError::from)?;
        Ok(Self { chat, allocation })
    }

    pub fn new(
        id: NativeOwned<String>,
        metadata: NativeOwned<super::ChatMetadata>,
        resume: NativeOwned<super::ResumeDescriptor>,
        transcript: NativeOwned<super::TranscriptData>,
        composer: NativeOwned<super::ComposerData>,
    ) -> Result<Self, HistoryCodecError> {
        let joined = id
            .join(metadata)
            .and_then(|v| v.join(resume))
            .and_then(|v| v.join(transcript))
            .and_then(|v| v.join(composer))
            .map_err(native_error)?;
        let (((((id, metadata), resume), transcript), composer), allocation) = joined.into_parts();
        let chat = SavedChat::from_parts(
            allocation.original_ledger().authority(),
            id,
            metadata,
            resume,
            transcript,
            composer,
        )?;
        Ok(Self { chat, allocation })
    }
    pub(crate) fn into_document(self) -> NativeOwned<super::SavedDocument> {
        let Self { chat, allocation } = self;
        NativeOwned::from_parts(chat.into_document(), allocation)
    }
    pub(crate) fn prepare_document(
        self,
        origin: HistoryOrigin,
        grammar: Option<super::retention::LegacyFormat>,
        source_resident: usize,
    ) -> Result<(NativeOwned<super::SavedDocument>, super::PreparedHistory), HistoryCodecError>
    {
        let Self { chat, allocation } = self;
        let chat = super::codec::prepare_in_ledger(
            chat,
            origin,
            grammar,
            source_resident,
            allocation.original_ledger(),
        )?;
        let (id, metadata, resume, transcript, composer, prepared) = chat.into_parts();
        let document = NativeOwned::from_parts(
            super::SavedDocument {
                id,
                metadata,
                resume,
                transcript,
                composer,
            },
            allocation,
        );
        let prepared = prepared.ok_or(HistoryCodecError::CandidateMismatch)?;
        Ok((document, prepared))
    }
    /// The complete typed graph and its native allocation token coexist through
    /// actual owner census, format policy, prepared caches and encoding decision.
    pub fn prepare(
        self,
        origin: HistoryOrigin,
        grammar: Option<super::retention::LegacyFormat>,
        source_resident: usize,
    ) -> Result<SavedChat, HistoryCodecError> {
        let Self { chat, allocation } = self;
        let prepared = super::codec::prepare_in_ledger(
            chat,
            origin,
            grammar,
            source_resident,
            allocation.original_ledger(),
        );
        // This is the operation's terminal ownership boundary: no more native
        // allocations occur after the phase token retires. Existing shared JSON,
        // identifier, display and prepared-cache reservations stay with owners.
        drop(allocation);
        prepared
    }
}
fn native_error(error: JsonDecodeError) -> HistoryCodecError {
    error.into()
}
