//! Controlled iterative literal JSON construction. Reserved keys are ordinary data.
//! Native readers/stock grammar/legacy raw expansion remain outside this owner.
mod containers;
mod facts;
mod frontier;
pub(super) mod native_object;
pub use facts::{LiteralTextFact, LiteralTextFacts};
pub use frontier::LiteralFrontier;
mod syntax;
pub use syntax::{LiteralSyntax, StringRole};
#[cfg(test)]
mod tests;
mod text;
use super::{
    DecodeLedger, DecodedJson, JsonDecodeError,
    ledger::Reservation,
    nodes::{next_capacity, product},
};
use crate::model::json::lifecycle::{Cleanup, GuardedValue};
use containers::Frame;
use serde_json::{Number, Value};
use std::mem::size_of;
pub use text::{ExactLiteralText, LiteralText};
type Managed = GuardedValue;
pub struct LiteralBuilder {
    ledger: DecodeLedger,
    frames: Vec<Frame>,
    stack: Reservation,
    root: Option<Managed>,
    cleanup: Option<Cleanup>,
    _fixed: Reservation,
}
impl LiteralBuilder {
    /// Unknown-frontier construction prepays local certified cleanup before each
    /// completed aggregate. Planned decoding reuses one upfront workspace.
    pub fn new(ledger: &DecodeLedger) -> Result<Self, JsonDecodeError> {
        Ok(Self {
            ledger: ledger.share(),
            frames: Vec::new(),
            stack: ledger.reserve(0)?,
            root: None,
            cleanup: None,
            _fixed: ledger.reserve(size_of::<Self>())?,
        })
    }
    /// The adapter authenticates a lexical upper bound; this private workspace
    /// capability is actually allocated and checked before any managed Value.
    pub fn with_cleanup_frontier(
        ledger: &DecodeLedger,
        frontier: usize,
    ) -> Result<Self, JsonDecodeError> {
        let mut builder = Self::new(ledger)?;
        builder.cleanup = Some(Cleanup::admit(frontier, ledger)?);
        Ok(builder)
    }
    pub fn text(&self) -> Result<LiteralText, JsonDecodeError> {
        LiteralText::new(&self.ledger)
    }
    pub fn exact_text(&self, decoded_bytes: usize) -> Result<ExactLiteralText, JsonDecodeError> {
        ExactLiteralText::new(&self.ledger, decoded_bytes)
    }
    pub fn key(&mut self, key: LiteralText) -> Result<(), JsonDecodeError> {
        if !key.belongs_to(&self.ledger) {
            return Err(JsonDecodeError::ForeignLedger);
        }
        self.frames
            .last_mut()
            .ok_or(JsonDecodeError::ConstructionOrder)?
            .key(key)
    }
    pub fn start_array(&mut self) -> Result<(), JsonDecodeError> {
        self.push_frame(Frame::array(&self.ledger)?)
    }
    pub fn start_array_exact(&mut self, count: usize) -> Result<(), JsonDecodeError> {
        self.push_frame(Frame::array_exact(&self.ledger, count)?)
    }
    pub fn start_object(&mut self) -> Result<(), JsonDecodeError> {
        self.push_frame(Frame::object(&self.ledger)?)
    }
    fn push_frame(&mut self, frame: Frame) -> Result<(), JsonDecodeError> {
        self.can_attach()?;
        if self.frames.len() == self.frames.capacity() {
            let capacity = next_capacity(self.frames.capacity())?;
            let next = self
                .ledger
                .reserve(product(capacity, size_of::<Frame>())?)?;
            self.frames
                .try_reserve_exact(capacity - self.frames.len())?;
            if self.frames.capacity() != capacity {
                return Err(JsonDecodeError::CapacityProfile);
            }
            self.stack = next;
        }
        self.frames.push(frame);
        Ok(())
    }
    pub fn end_container(&mut self) -> Result<(), JsonDecodeError> {
        let frame = self
            .frames
            .pop()
            .ok_or(JsonDecodeError::ConstructionOrder)?;
        let child = frame.finish(&self.ledger, self.cleanup.as_ref())?;
        self.attach(child)
    }
    pub fn null(&mut self) -> Result<(), JsonDecodeError> {
        self.scalar(Value::Null)
    }
    pub fn boolean(&mut self, value: bool) -> Result<(), JsonDecodeError> {
        self.scalar(Value::Bool(value))
    }
    pub fn number(&mut self, value: Number) -> Result<(), JsonDecodeError> {
        self.scalar(Value::Number(value))
    }
    pub fn string(&mut self, text: LiteralText) -> Result<(), JsonDecodeError> {
        let (value, allocation) = text.into_parts(&self.ledger)?;
        self.attach(GuardedValue::new(
            Value::String(value),
            1,
            self.cleanup.clone(),
            allocation,
        ))
    }
    fn scalar(&mut self, value: Value) -> Result<(), JsonDecodeError> {
        self.attach(GuardedValue::new(
            value,
            1,
            self.cleanup.clone(),
            self.ledger.reserve(0)?,
        ))
    }
    fn can_attach(&self) -> Result<(), JsonDecodeError> {
        match self.frames.last() {
            Some(frame) if frame.accepts_value() => Ok(()),
            None if self.root.is_none() => Ok(()),
            _ => Err(JsonDecodeError::ConstructionOrder),
        }
    }
    fn attach(&mut self, value: Managed) -> Result<(), JsonDecodeError> {
        self.can_attach()?;
        match self.frames.last_mut() {
            Some(frame) => frame.push(value, &self.ledger),
            None => {
                self.root = Some(value);
                Ok(())
            }
        }
    }
    /// Fresh native schema traversal, without publishing shared JSON aliases.
    pub fn finish_native(mut self) -> Result<super::native::NativeNode, JsonDecodeError> {
        if !self.frames.is_empty() {
            return Err(JsonDecodeError::ConstructionOrder);
        }
        let root = self.root.take().ok_or(JsonDecodeError::ConstructionOrder)?;
        let root = if root.native_cleanup().is_none() {
            let cleanup = Cleanup::admit(root.frontier(), &self.ledger)?;
            root.reparent(&cleanup)?
        } else {
            root
        };
        super::native::NativeNode::capture(root, &self.ledger)
    }
    pub fn finish(mut self) -> Result<DecodedJson, JsonDecodeError> {
        if !self.frames.is_empty() {
            return Err(JsonDecodeError::ConstructionOrder);
        }
        let root = self.root.take().ok_or(JsonDecodeError::ConstructionOrder)?;
        let owned = super::super::OwnedJson::from_admitted(root, &self.ledger)?;
        Ok(DecodedJson { value: owned })
    }
}
