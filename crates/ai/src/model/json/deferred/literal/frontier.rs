//! Lexical VALUE frontier upper bound, not exact deduplicated object residency.
use super::super::{
    DecodeLedger, JsonDecodeError,
    ledger::Reservation,
    nodes::{next_capacity, product},
};
use std::mem::size_of;
struct Container {
    children: usize,
    maximum: usize,
}
/// Keys never enter the cleanup worklist. Repeated object values conservatively
/// keep lexical indices, which dominate their final first-insertion unique index.
pub struct LiteralFrontier {
    frames: Vec<Container>,
    root: Option<usize>,
    ledger: DecodeLedger,
    backing: Reservation,
    _fixed: Reservation,
}
impl LiteralFrontier {
    pub fn new(ledger: &DecodeLedger) -> Result<Self, JsonDecodeError> {
        Ok(Self {
            frames: Vec::new(),
            root: None,
            ledger: ledger.share(),
            backing: ledger.reserve(0)?,
            _fixed: ledger.reserve(size_of::<Self>())?,
        })
    }
    pub fn start(&mut self) -> Result<(), JsonDecodeError> {
        if self.frames.len() == self.frames.capacity() {
            let capacity = next_capacity(self.frames.capacity())?;
            let allocation = self
                .ledger
                .reserve(product(capacity, size_of::<Container>())?)?;
            self.frames
                .try_reserve_exact(capacity - self.frames.len())?;
            if self.frames.capacity() != capacity {
                return Err(JsonDecodeError::CapacityProfile);
            }
            self.backing = allocation;
        }
        self.frames.push(Container {
            children: 0,
            maximum: 1,
        });
        Ok(())
    }
    pub fn scalar(&mut self) -> Result<(), JsonDecodeError> {
        self.attach(1)
    }
    pub fn end(&mut self) -> Result<(), JsonDecodeError> {
        let container = self
            .frames
            .pop()
            .ok_or(JsonDecodeError::ConstructionOrder)?;
        self.attach(container.maximum.max(container.children))
    }
    fn attach(&mut self, bound: usize) -> Result<(), JsonDecodeError> {
        if let Some(parent) = self.frames.last_mut() {
            parent.maximum = parent.maximum.max(
                bound
                    .checked_add(parent.children)
                    .ok_or(JsonDecodeError::Arithmetic)?,
            );
            parent.children = parent
                .children
                .checked_add(1)
                .ok_or(JsonDecodeError::Arithmetic)?;
        } else if self.root.replace(bound).is_some() {
            return Err(JsonDecodeError::ConstructionOrder);
        }
        Ok(())
    }
    pub fn finish(self) -> Result<usize, JsonDecodeError> {
        if !self.frames.is_empty() {
            return Err(JsonDecodeError::ConstructionOrder);
        }
        self.root.ok_or(JsonDecodeError::ConstructionOrder)
    }
}
#[cfg(test)]
mod tests;
