//! Same-ledger guarded object reconstruction never exposes bare child payloads.
use super::{DecodeLedger, JsonDecodeError, containers::Frame, text::LiteralText};
use crate::model::json::deferred::{
    ledger::Reservation,
    native::{NativeNode, NativeText},
};
use std::mem::size_of;

pub struct NativeObject {
    frame: Frame,
    ledger: DecodeLedger,
    remaining: usize,
    _fixed: Reservation,
}
impl NativeObject {
    /// Capacity bounds the number of supplied entries; duplicate keys retain
    /// first position and last value, just like the controlled literal builder.
    pub fn new(ledger: &DecodeLedger, capacity: usize) -> Result<Self, JsonDecodeError> {
        let fixed = ledger.reserve(size_of::<Self>())?;
        let frame = Frame::object_exact(ledger, capacity)?;
        Ok(Self {
            frame,
            ledger: ledger.share(),
            remaining: capacity,
            _fixed: fixed,
        })
    }
    pub fn push(&mut self, key: NativeText, value: NativeNode) -> Result<(), JsonDecodeError> {
        if self.remaining == 0 {
            return Err(JsonDecodeError::ConstructionOrder);
        }
        let key = LiteralText::from_native(key, &self.ledger)?;
        let value = value.into_guarded(&self.ledger)?;
        self.frame.key(key)?;
        self.frame.push(value, &self.ledger)?;
        self.remaining -= 1;
        Ok(())
    }
    pub fn finish(self) -> Result<NativeNode, JsonDecodeError> {
        NativeNode::capture(self.frame.finish_native(&self.ledger)?, &self.ledger)
    }
}
