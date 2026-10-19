//! Pure final transcript-row construction from authentic controlled text.
use crate::{
    model::json::deferred::{
        DecodeLedger, JsonDecodeError, ledger::Reservation, literal::LiteralText,
    },
    thread::Entry,
};
use std::mem::size_of;
mod display;
mod tool;
pub use display::{DecodedDisplay, DecodedDisplayOutcome};
mod transcript;
pub use tool::ToolRowHeader;
pub use transcript::{DecodedTranscript, TranscriptBuildError};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavedRowKind {
    User,
    Agent,
    Thought,
    Content,
    Tool,
}
impl SavedRowKind {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "user" => Some(Self::User),
            "agent" => Some(Self::Agent),
            "thought" => Some(Self::Thought),
            "content" => Some(Self::Content),
            "tool" => Some(Self::Tool),
            _ => None,
        }
    }
}
/// Entry is dropped before its authentic backing/fixed reservations. No mutable
/// Entry/String or fabricated allocation receipt escapes this construction owner.
pub struct DecodedRow {
    entry: Entry,
    _backing: Reservation,
    _fixed: Reservation,
}
impl DecodedRow {
    pub fn text(
        kind: SavedRowKind,
        text: LiteralText,
        ledger: &DecodeLedger,
    ) -> Result<Self, JsonDecodeError> {
        if !matches!(kind, SavedRowKind::Agent | SavedRowKind::Thought) {
            return Err(JsonDecodeError::ConstructionOrder);
        }
        let fixed = ledger.reserve(size_of::<Self>())?;
        let (text, backing) = text.into_parts(ledger)?;
        let entry = match kind {
            SavedRowKind::Agent => Entry::Agent(text),
            SavedRowKind::Thought => Entry::Thought(text),
            _ => return Err(JsonDecodeError::ConstructionOrder),
        };
        Ok(Self {
            entry,
            _backing: backing,
            _fixed: fixed,
        })
    }
    pub fn as_entry(&self) -> &Entry {
        &self.entry
    }
}
fn owned_text(
    text: LiteralText,
    ledger: &DecodeLedger,
) -> Result<crate::model::native::NativeOwned<String>, JsonDecodeError> {
    let (text, allocation) = text.into_parts(ledger)?;
    Ok(crate::model::native::NativeOwned::from_parts(
        text, allocation,
    ))
}
fn owned_json(
    value: crate::model::json::deferred::DecodedJson,
    ledger: &DecodeLedger,
) -> Result<crate::model::native::NativeOwned<crate::model::json::OwnedJson>, JsonDecodeError> {
    let allocation = ledger.reserve(0)?;
    Ok(crate::model::native::NativeOwned::from_parts(
        value.into_owned(ledger)?,
        allocation,
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn final_text_row_moves_controlled_buffer_and_keeps_reservation_until_drop() {
        let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
        let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
        let baseline = ledger.live_requested_bytes();
        let mut text = LiteralText::new(&ledger).unwrap();
        text.append("  literal 日本  ").unwrap();
        let row = DecodedRow::text(SavedRowKind::Thought, text, &ledger).unwrap();
        assert!(ledger.live_requested_bytes() > baseline);
        assert_eq!(row.as_entry(), &Entry::Thought("  literal 日本  ".into()));
        drop(row);
        assert_eq!(ledger.live_requested_bytes(), baseline);
    }
    #[test]
    fn final_text_row_rejects_foreign_ledger_without_adopting_untracked_text() {
        let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
        let original = DecodeLedger::new(&authority, 4096).unwrap();
        let foreign = DecodeLedger::new(&authority, 4096).unwrap();
        let baseline = foreign.live_requested_bytes();
        let mut text = LiteralText::new(&original).unwrap();
        text.append("original").unwrap();
        assert!(matches!(
            DecodedRow::text(SavedRowKind::Agent, text, &foreign),
            Err(JsonDecodeError::ForeignLedger)
        ));
        assert_eq!(foreign.live_requested_bytes(), baseline);
    }
}
