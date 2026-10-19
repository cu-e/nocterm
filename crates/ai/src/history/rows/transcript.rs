//! Controlled final-row accumulation; authentic row receipts move into this owner.
use super::{DecodeLedger, DecodedRow, JsonDecodeError, Reservation, size_of};
use crate::{history::MAX_SAVED_ENTRIES, thread::Entry};
#[derive(Debug, thiserror::Error)]
pub enum TranscriptBuildError {
    #[error("Saved transcript exceeds its entry limit")]
    Entries,
    #[error("Saved transcript controlled allocation failed: {0}")]
    Json(#[from] JsonDecodeError),
}
/// Rows drop before both vector/fixed backing receipts. No mutable/raw Entry Vec
/// can escape while its construction provenance is discarded.
pub struct DecodedTranscript {
    rows: Vec<DecodedRow>,
    backing: Reservation,
    ledger: DecodeLedger,
    _fixed: Reservation,
}
impl DecodedTranscript {
    pub fn new(ledger: &DecodeLedger) -> Result<Self, JsonDecodeError> {
        Ok(Self {
            rows: Vec::new(),
            backing: ledger.reserve(0)?,
            ledger: ledger.share(),
            _fixed: ledger.reserve(size_of::<Self>())?,
        })
    }
    /// Call before decoding another row; no2001st row payload need be allocated.
    pub fn check_next(&self) -> Result<(), TranscriptBuildError> {
        if self.rows.len() >= MAX_SAVED_ENTRIES {
            Err(TranscriptBuildError::Entries)
        } else {
            Ok(())
        }
    }
    pub fn push(&mut self, row: DecodedRow) -> Result<(), TranscriptBuildError> {
        self.check_next()?;
        if !row._backing.belongs_to(&self.ledger) {
            return Err(JsonDecodeError::ForeignLedger.into());
        }
        if self.rows.len() == self.rows.capacity() {
            let capacity = self
                .rows
                .capacity()
                .checked_mul(2)
                .ok_or(JsonDecodeError::Arithmetic)?
                .clamp(4, MAX_SAVED_ENTRIES);
            let bytes = capacity
                .checked_mul(size_of::<DecodedRow>())
                .ok_or(JsonDecodeError::Arithmetic)?;
            let next = self.ledger.reserve(bytes)?;
            self.rows
                .try_reserve_exact(capacity - self.rows.len())
                .map_err(JsonDecodeError::from)?;
            if self.rows.capacity() != capacity {
                return Err(JsonDecodeError::CapacityProfile.into());
            }
            self.backing = next;
        }
        self.rows.push(row);
        Ok(())
    }
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    pub fn get(&self, index: usize) -> Option<&Entry> {
        self.rows.get(index).map(DecodedRow::as_entry)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{history::SavedRowKind, model::json::deferred::literal::LiteralText};
    #[test]
    fn final_transcript_adopts_real_rows_and_releases_all_retained_backing_on_drop() {
        let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
        let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
        let initial = ledger.live_requested_bytes();
        let mut rows = DecodedTranscript::new(&ledger).unwrap();
        let mut text = LiteralText::new(&ledger).unwrap();
        text.append("original").unwrap();
        let row = DecodedRow::text(SavedRowKind::Agent, text, &ledger).unwrap();
        rows.push(row).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows.get(0), Some(&Entry::Agent("original".into())));
        assert!(ledger.live_requested_bytes() > initial);
        drop(rows);
        assert_eq!(ledger.live_requested_bytes(), initial);
    }
    #[test]
    fn final_transcript_rejects_next_record_before_payload_construction_at_original_limit() {
        let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
        let ledger = DecodeLedger::new(&authority, 4 * 1024 * 1024).unwrap();
        let mut rows = DecodedTranscript::new(&ledger).unwrap();
        for _ in 0..MAX_SAVED_ENTRIES {
            rows.check_next().unwrap();
            let text = LiteralText::new(&ledger).unwrap();
            rows.push(DecodedRow::text(SavedRowKind::Agent, text, &ledger).unwrap())
                .unwrap();
        }
        let before = ledger.live_requested_bytes();
        assert!(matches!(
            rows.check_next(),
            Err(TranscriptBuildError::Entries)
        ));
        assert_eq!(ledger.live_requested_bytes(), before);
        assert_eq!(rows.len(), MAX_SAVED_ENTRIES);
    }
}
