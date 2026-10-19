//! Actual parser frames prepay replacement backing before growing their Vec.
use super::ValueError;
use nocterm_ai::model::json::deferred::JsonDecodeError;
use nocterm_ai::model::json::deferred::{DecodeLedger, NativeScratch};
use std::mem::size_of;
pub(super) struct Frames<'a, T> {
    values: Vec<T>,
    allocation: NativeScratch,
    _fixed: NativeScratch,
    ledger: &'a DecodeLedger,
}
impl<'a, T> Frames<'a, T> {
    pub(super) fn new(ledger: &'a DecodeLedger) -> Result<Self, ValueError> {
        let fixed = NativeScratch::reserve(ledger, size_of::<Self>())?;
        Ok(Self {
            values: Vec::new(),
            allocation: NativeScratch::reserve(ledger, 0)?,
            _fixed: fixed,
            ledger,
        })
    }
    pub(super) fn push(&mut self, value: T) -> Result<(), ValueError> {
        if self.values.len() == self.values.capacity() {
            let capacity = self
                .values
                .capacity()
                .checked_mul(2)
                .ok_or(JsonDecodeError::Arithmetic)?
                .max(4);
            let bytes = capacity
                .checked_mul(size_of::<T>())
                .ok_or(JsonDecodeError::Arithmetic)?;
            let replacement = NativeScratch::reserve(self.ledger, bytes)?;
            self.values
                .try_reserve_exact(capacity - self.values.len())
                .map_err(JsonDecodeError::from)?;
            if self.values.capacity() != capacity {
                return Err(JsonDecodeError::CapacityProfile.into());
            }
            self.allocation = replacement;
        }
        self.values.push(value);
        Ok(())
    }
    pub(super) fn reverse(&mut self) {
        self.values.reverse();
    }
    pub(super) fn len(&self) -> usize {
        self.values.len()
    }
    pub(super) fn get(&self, index: usize) -> Option<&T> {
        self.values.get(index)
    }
    pub(super) fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.values.get_mut(index)
    }
    pub(super) fn pop(&mut self) -> Option<T> {
        self.values.pop()
    }
}
