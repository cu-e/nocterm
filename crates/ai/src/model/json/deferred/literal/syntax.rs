//! Pure structural token transitions. Stock serde owns leaf lexical acceptance.
use super::super::{
    DecodeLedger, JsonDecodeError,
    ledger::Reservation,
    nodes::{next_capacity, product},
};
use std::mem::size_of;
#[derive(Clone, Copy)]
enum State {
    ArrayFirst,
    ArrayValue,
    ArrayAfter,
    ObjectFirst,
    ObjectKey,
    ObjectColon,
    ObjectValue,
    ObjectAfter,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringRole {
    Key,
    Value,
}
/// A separate explicitly charged syntax stack, never an implicit Rust recursion.
pub struct LiteralSyntax {
    ledger: DecodeLedger,
    frames: Vec<State>,
    backing: Reservation,
    root: bool,
    _fixed: Reservation,
}
impl LiteralSyntax {
    pub fn new(ledger: &DecodeLedger) -> Result<Self, JsonDecodeError> {
        Ok(Self {
            ledger: ledger.share(),
            frames: Vec::new(),
            backing: ledger.reserve(0)?,
            root: false,
            _fixed: ledger.reserve(size_of::<Self>())?,
        })
    }
    fn value(&mut self) -> Result<(), JsonDecodeError> {
        match self.frames.last_mut() {
            Some(state @ (State::ArrayFirst | State::ArrayValue)) => *state = State::ArrayAfter,
            Some(state @ State::ObjectValue) => *state = State::ObjectAfter,
            None if !self.root => self.root = true,
            _ => return Err(JsonDecodeError::ConstructionOrder),
        }
        Ok(())
    }
    pub fn scalar(&mut self) -> Result<(), JsonDecodeError> {
        self.value()
    }
    pub fn string(&mut self) -> Result<StringRole, JsonDecodeError> {
        match self.frames.last_mut() {
            Some(state @ (State::ObjectFirst | State::ObjectKey)) => {
                *state = State::ObjectColon;
                Ok(StringRole::Key)
            }
            _ => {
                self.value()?;
                Ok(StringRole::Value)
            }
        }
    }
    pub fn start_array(&mut self) -> Result<(), JsonDecodeError> {
        self.start(State::ArrayFirst)
    }
    pub fn start_object(&mut self) -> Result<(), JsonDecodeError> {
        self.start(State::ObjectFirst)
    }
    fn start(&mut self, state: State) -> Result<(), JsonDecodeError> {
        self.value()?;
        if self.frames.len() == self.frames.capacity() {
            let capacity = next_capacity(self.frames.capacity())?;
            let next = self
                .ledger
                .reserve(product(capacity, size_of::<State>())?)?;
            self.frames
                .try_reserve_exact(capacity - self.frames.len())?;
            if self.frames.capacity() != capacity {
                return Err(JsonDecodeError::CapacityProfile);
            }
            self.backing = next;
        }
        self.frames.push(state);
        Ok(())
    }
    pub fn end_array(&mut self) -> Result<(), JsonDecodeError> {
        match self.frames.last() {
            Some(State::ArrayFirst | State::ArrayAfter) => {
                self.frames.pop();
                Ok(())
            }
            _ => Err(JsonDecodeError::ConstructionOrder),
        }
    }
    pub fn end_object(&mut self) -> Result<(), JsonDecodeError> {
        match self.frames.last() {
            Some(State::ObjectFirst | State::ObjectAfter) => {
                self.frames.pop();
                Ok(())
            }
            _ => Err(JsonDecodeError::ConstructionOrder),
        }
    }
    pub fn comma(&mut self) -> Result<(), JsonDecodeError> {
        match self.frames.last_mut() {
            Some(state @ State::ArrayAfter) => *state = State::ArrayValue,
            Some(state @ State::ObjectAfter) => *state = State::ObjectKey,
            _ => return Err(JsonDecodeError::ConstructionOrder),
        };
        Ok(())
    }
    pub fn colon(&mut self) -> Result<(), JsonDecodeError> {
        match self.frames.last_mut() {
            Some(state @ State::ObjectColon) => {
                *state = State::ObjectValue;
                Ok(())
            }
            _ => Err(JsonDecodeError::ConstructionOrder),
        }
    }
    pub fn finish(self) -> Result<(), JsonDecodeError> {
        if self.root && self.frames.is_empty() {
            Ok(())
        } else {
            Err(JsonDecodeError::ConstructionOrder)
        }
    }
}
