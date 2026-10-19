//! Iterative literal containers hold controlled child values and exact backing.
use super::super::{
    DecodeLedger, JsonDecodeError,
    ledger::Reservation,
    nodes::{next_capacity, object_layout, product},
};
use super::{Managed, text::LiteralText};
use crate::model::json::lifecycle::{Cleanup, GuardedValue};
use indexmap::IndexMap;
use serde_json::{Map, Value};
use std::mem::size_of;
#[expect(
    clippy::large_enum_variant,
    reason = "the frame stack is charged per inline frame; boxing would add unpaid allocations"
)]
pub(super) enum Frame {
    Array(ArrayFrame),
    Object(ObjectFrame),
}
pub(super) struct ArrayFrame {
    values: Vec<Managed>,
    backing: Reservation,
    exact: Option<usize>,
}
pub(super) struct ObjectFrame {
    values: IndexMap<String, Managed>,
    backing: Reservation,
    keys: Reservation,
    pending: Option<LiteralText>,
    capacity: usize,
}
impl Frame {
    pub(super) fn array(ledger: &DecodeLedger) -> Result<Self, JsonDecodeError> {
        Ok(Self::Array(ArrayFrame {
            values: Vec::new(),
            backing: ledger.reserve(0)?,
            exact: None,
        }))
    }
    pub(super) fn array_exact(
        ledger: &DecodeLedger,
        count: usize,
    ) -> Result<Self, JsonDecodeError> {
        let backing = ledger.reserve(product(count, size_of::<Managed>())?)?;
        let mut values = Vec::new();
        values.try_reserve_exact(count)?;
        if values.capacity() != count {
            return Err(JsonDecodeError::CapacityProfile);
        }
        Ok(Self::Array(ArrayFrame {
            values,
            backing,
            exact: Some(count),
        }))
    }
    pub(super) fn object(ledger: &DecodeLedger) -> Result<Self, JsonDecodeError> {
        Ok(Self::Object(ObjectFrame {
            values: IndexMap::new(),
            backing: ledger.reserve(0)?,
            keys: ledger.reserve(0)?,
            pending: None,
            capacity: 0,
        }))
    }
    pub(super) fn object_exact(
        ledger: &DecodeLedger,
        capacity: usize,
    ) -> Result<Self, JsonDecodeError> {
        let backing = ledger.reserve(object_layout::<Managed>(capacity)?)?;
        let mut values = IndexMap::new();
        values.try_reserve_exact(capacity)?;
        if values.capacity() != capacity {
            return Err(JsonDecodeError::CapacityProfile);
        }
        Ok(Self::Object(ObjectFrame {
            values,
            backing,
            keys: ledger.reserve(0)?,
            pending: None,
            capacity,
        }))
    }
    pub(super) fn finish_native(self, ledger: &DecodeLedger) -> Result<Managed, JsonDecodeError> {
        match self {
            Self::Object(object) => object.finish_reparented(ledger),
            Self::Array(array) => array.finish_reparented(ledger),
        }
    }
    pub(super) fn accepts_value(&self) -> bool {
        match self {
            Self::Array(array) => array.exact.is_none_or(|count| array.values.len() < count),
            Self::Object(object) => object.pending.is_some(),
        }
    }
    pub(super) fn key(&mut self, text: LiteralText) -> Result<(), JsonDecodeError> {
        match self {
            Self::Object(object) if object.pending.is_none() => {
                object.pending = Some(text);
                Ok(())
            }
            _ => Err(JsonDecodeError::ConstructionOrder),
        }
    }
    pub(super) fn push(
        &mut self,
        value: Managed,
        ledger: &DecodeLedger,
    ) -> Result<(), JsonDecodeError> {
        match self {
            Self::Array(array) => array.push(value, ledger),
            Self::Object(object) => object.push(value, ledger),
        }
    }
    pub(super) fn finish(
        self,
        ledger: &DecodeLedger,
        cleanup: Option<&Cleanup>,
    ) -> Result<Managed, JsonDecodeError> {
        match (self, cleanup) {
            (frame, None) => frame.finish_native(ledger),
            (Self::Array(array), Some(cleanup)) => array.finish(ledger, Some(cleanup)),
            (Self::Object(object), Some(cleanup)) => object.finish(ledger, Some(cleanup)),
        }
    }
}
impl ArrayFrame {
    fn check_complete(&self) -> Result<(), JsonDecodeError> {
        if self.exact.is_some_and(|count| self.values.len() != count) {
            return Err(JsonDecodeError::ConstructionOrder);
        }
        Ok(())
    }
    fn finish_reparented(self, ledger: &DecodeLedger) -> Result<Managed, JsonDecodeError> {
        self.check_complete()?;
        let bound = child_bound(self.values.iter().map(|child| child.frontier()))?;
        let cleanup = Cleanup::admit(bound, ledger)?;
        let allocation = ledger.reserve(product(self.values.len(), size_of::<Value>())?)?;
        let mut values = Vec::new();
        values.try_reserve_exact(self.values.len())?;
        if values.capacity() != self.values.len() {
            return Err(JsonDecodeError::CapacityProfile);
        }
        let mut guarded = GuardedValue::new(
            Value::Array(values),
            bound,
            Some(cleanup.clone()),
            allocation,
        );
        for child in self.values {
            guarded.push_array(child.reparent(&cleanup)?)?;
        }
        drop(self.backing);
        Ok(guarded)
    }
    fn push(&mut self, value: Managed, ledger: &DecodeLedger) -> Result<(), JsonDecodeError> {
        if self.exact.is_some_and(|count| self.values.len() >= count) {
            return Err(JsonDecodeError::ConstructionOrder);
        }
        if self.values.len() == self.values.capacity() {
            let capacity = next_capacity(self.values.capacity())?;
            let next = ledger.reserve(product(capacity, size_of::<Managed>())?)?;
            self.values
                .try_reserve_exact(capacity - self.values.len())?;
            if self.values.capacity() != capacity {
                return Err(JsonDecodeError::CapacityProfile);
            }
            self.backing = next;
        }
        self.values.push(value);
        Ok(())
    }
    fn finish(
        self,
        ledger: &DecodeLedger,
        cleanup: Option<&Cleanup>,
    ) -> Result<Managed, JsonDecodeError> {
        self.check_complete()?;
        let bound = child_bound(self.values.iter().map(|child| child.frontier()))?;
        if let Some(cleanup) = cleanup {
            cleanup.check(bound)?;
        }
        let allocation = ledger.reserve(product(self.values.len(), size_of::<Value>())?)?;
        let mut values = Vec::new();
        values.try_reserve_exact(self.values.len())?;
        if values.capacity() != self.values.len() {
            return Err(JsonDecodeError::CapacityProfile);
        }
        let mut guarded =
            GuardedValue::new(Value::Array(values), bound, cleanup.cloned(), allocation);
        for child in self.values {
            guarded.push_array(child)?;
        }
        drop(self.backing);
        Ok(guarded)
    }
}
impl ObjectFrame {
    fn finish_reparented(mut self, ledger: &DecodeLedger) -> Result<Managed, JsonDecodeError> {
        if self.pending.is_some() {
            return Err(JsonDecodeError::ConstructionOrder);
        }
        let bound = child_bound(self.values.values().map(|child| child.frontier()))?;
        let cleanup = Cleanup::admit(bound, ledger)?;
        let mut allocation = ledger.reserve(object_layout::<Value>(self.values.len())?)?;
        allocation.merge_from(&mut self.keys)?;
        let values = Map::with_capacity(self.values.len());
        let mut guarded = GuardedValue::new(
            Value::Object(values),
            bound,
            Some(cleanup.clone()),
            allocation,
        );
        for (key, child) in self.values {
            guarded.insert_object(key, child.reparent(&cleanup)?)?;
        }
        drop(self.backing);
        Ok(guarded)
    }
    fn push(&mut self, value: Managed, ledger: &DecodeLedger) -> Result<(), JsonDecodeError> {
        let text = self
            .pending
            .take()
            .ok_or(JsonDecodeError::ConstructionOrder)?;
        let (key, key_allocation) = text.into_parts(ledger)?;
        if self.values.contains_key(&key) {
            // Retain original key position/storage; replaced descendants retire now.
            drop(self.values.insert(key, value));
            drop(key_allocation);
        } else {
            if self.values.len() == self.capacity {
                let capacity = next_capacity(self.capacity)?;
                let next = ledger.reserve(object_layout::<Managed>(capacity)?)?;
                self.values
                    .try_reserve_exact(capacity - self.values.len())?;
                if self.values.capacity() != capacity {
                    return Err(JsonDecodeError::CapacityProfile);
                }
                self.backing = next;
                self.capacity = capacity;
            }
            self.keys.merge(key_allocation)?;
            self.values.insert(key, value);
        }
        Ok(())
    }
    fn finish(
        mut self,
        ledger: &DecodeLedger,
        cleanup: Option<&Cleanup>,
    ) -> Result<Managed, JsonDecodeError> {
        if self.pending.is_some() {
            return Err(JsonDecodeError::ConstructionOrder);
        }
        let bound = child_bound(self.values.values().map(|child| child.frontier()))?;
        if let Some(cleanup) = cleanup {
            cleanup.check(bound)?;
        }
        let mut allocation = ledger.reserve(object_layout::<Value>(self.values.len())?)?;
        allocation.merge_from(&mut self.keys)?;
        let values = Map::with_capacity(self.values.len());
        let mut guarded =
            GuardedValue::new(Value::Object(values), bound, cleanup.cloned(), allocation);
        for (key, child) in self.values {
            guarded.insert_object(key, child)?;
        }
        drop(self.backing);
        Ok(guarded)
    }
}

/// One linear pass over final unique children, before any raw aggregate exists.
fn child_bound(children: impl Iterator<Item = usize>) -> Result<usize, JsonDecodeError> {
    let mut count = 0usize;
    let mut maximum = 1;
    for bound in children {
        maximum = maximum.max(
            bound
                .checked_add(count)
                .ok_or(JsonDecodeError::Arithmetic)?,
        );
        count = count.checked_add(1).ok_or(JsonDecodeError::Arithmetic)?;
    }
    Ok(maximum.max(count))
}
