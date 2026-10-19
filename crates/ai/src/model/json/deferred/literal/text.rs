//! Controlled streaming UTF-8 text; callers supply already validated fragments.
use super::super::{
    DecodeLedger, JsonDecodeError,
    ledger::Reservation,
    nodes::{next_capacity, sum},
};
use std::mem::size_of;
/// Opaque ownership: no mutable String/Value or fabricated capacity receipt escapes.
pub struct LiteralText {
    text: String,
    backing: Reservation,
    ledger: DecodeLedger,
    _fixed: Reservation,
}
impl LiteralText {
    pub fn new(ledger: &DecodeLedger) -> Result<Self, JsonDecodeError> {
        Ok(Self {
            text: String::new(),
            backing: ledger.reserve(0)?,
            ledger: ledger.share(),
            _fixed: ledger.reserve(size_of::<Self>())?,
        })
    }
    pub fn as_str(&self) -> &str {
        &self.text
    }
    pub(super) fn from_native(
        text: crate::model::json::lifecycle::NativeText,
        ledger: &DecodeLedger,
    ) -> Result<Self, JsonDecodeError> {
        if !text.belongs_to(ledger) {
            return Err(JsonDecodeError::ForeignLedger);
        }
        let fixed = ledger.reserve(size_of::<Self>())?;
        let (text, backing) = text.into_parts();
        Ok(Self {
            text,
            backing,
            ledger: ledger.share(),
            _fixed: fixed,
        })
    }
    pub fn append(&mut self, fragment: &str) -> Result<(), JsonDecodeError> {
        let required = sum(self.text.len(), fragment.len())?;
        if required > self.text.capacity() {
            let mut capacity = next_capacity(self.text.capacity())?;
            while capacity < required {
                capacity = next_capacity(capacity)?;
            }
            let next = self.ledger.reserve(capacity)?;
            // Old backing remains charged until reallocation has completed.
            self.text.try_reserve_exact(capacity - self.text.len())?;
            if self.text.capacity() != capacity {
                return Err(JsonDecodeError::CapacityProfile);
            }
            self.backing = next;
        }
        self.text.push_str(fragment);
        Ok(())
    }
    pub(super) fn belongs_to(&self, ledger: &DecodeLedger) -> bool {
        self.backing.belongs_to(ledger)
    }
    pub(crate) fn into_parts(
        self,
        ledger: &DecodeLedger,
    ) -> Result<(String, Reservation), JsonDecodeError> {
        if !self.backing.belongs_to(ledger) {
            return Err(JsonDecodeError::ForeignLedger);
        }
        Ok((self.text, self.backing))
    }
}

/// Exact-capacity streaming text. This is allocation control, not a claim that
/// a caller-supplied length authenticates any source. Adapters must independently
/// authenticate their measurement and the bytes consumed during construction.
pub struct ExactLiteralText {
    text: LiteralText,
    expected: usize,
    _fixed: Reservation,
}
impl ExactLiteralText {
    pub fn new(ledger: &DecodeLedger, expected: usize) -> Result<Self, JsonDecodeError> {
        let fixed = ledger.reserve(size_of::<Self>() - size_of::<LiteralText>())?;
        let mut text = LiteralText::new(ledger)?;
        let backing = ledger.reserve(expected)?;
        text.text.try_reserve_exact(expected)?;
        if text.text.capacity() != expected {
            return Err(JsonDecodeError::CapacityProfile);
        }
        text.backing = backing;
        Ok(Self {
            text,
            expected,
            _fixed: fixed,
        })
    }
    pub fn append(&mut self, fragment: &str) -> Result<(), JsonDecodeError> {
        let actual = sum(self.text.text.len(), fragment.len())?;
        if actual > self.expected {
            return Err(JsonDecodeError::TextLength {
                expected: self.expected,
                actual,
            });
        }
        // The full backing was reserved before the first fragment. This checked
        // append cannot enter LiteralText's growth branch, even if replay changed.
        self.text.append(fragment)
    }
    pub fn finish(self) -> Result<LiteralText, JsonDecodeError> {
        let actual = self.text.text.len();
        if actual != self.expected {
            return Err(JsonDecodeError::TextLength {
                expected: self.expected,
                actual,
            });
        }
        Ok(self.text)
    }
}
