//! Flat facts are computed only from the fresh, private controlled root.
//! Both the retained facts and temporary borrowed walk are paid before growth.
use super::super::{
    DecodeLedger, JsonDecodeError,
    ledger::Reservation,
    nodes::{next_capacity, object_layout, product},
};
use crate::model::json::lifecycle::GuardedValue;
use crate::model::json::lifetime::Shared;
use serde_json::Value;
use std::mem::size_of;

pub(super) struct Fact {
    pub(super) bytes: usize,
    pub(super) frontier: usize,
    pub(super) key_bytes: usize,
    pub(super) first: Option<usize>,
    pub(super) next: Option<usize>,
    last: Option<usize>,
}
pub(super) struct Layout {
    facts: Vec<Fact>,
    _allocation: Reservation,
}
enum Visit<'a> {
    Enter {
        value: &'a Value,
        parent: Option<usize>,
        key_bytes: usize,
    },
    Exit(usize),
}
struct Metered<T> {
    values: Vec<T>,
    allocation: Reservation,
}
impl<T> Metered<T> {
    fn new(ledger: &DecodeLedger) -> Result<Self, JsonDecodeError> {
        Ok(Self {
            values: Vec::new(),
            allocation: ledger.reserve(0)?,
        })
    }
    fn push(&mut self, value: T, ledger: &DecodeLedger) -> Result<(), JsonDecodeError> {
        if self.values.len() == self.values.capacity() {
            let capacity = next_capacity(self.values.capacity())?;
            // Both old and replacement buffers remain paid through reallocation.
            let replacement = ledger.reserve(product(capacity, size_of::<T>())?)?;
            self.values
                .try_reserve_exact(capacity - self.values.len())?;
            if self.values.capacity() != capacity {
                return Err(JsonDecodeError::CapacityProfile);
            }
            self.allocation = replacement;
        }
        self.values.push(value);
        Ok(())
    }
}
impl Layout {
    pub(super) fn fact(&self, index: usize) -> &Fact {
        &self.facts[index]
    }
    pub(super) fn capture(
        value: &GuardedValue,
        ledger: &DecodeLedger,
    ) -> Result<Shared<Self>, JsonDecodeError> {
        if !value.belongs_to(ledger) {
            return Err(JsonDecodeError::ForeignLedger);
        }
        let fixed = ledger.reserve(crate::model::allocations::shared_inline_bytes::<Self>())?;
        let mut facts = Metered::new(ledger)?;
        let mut work = Metered::new(ledger)?;
        work.push(
            Visit::Enter {
                value: value.native_value(),
                parent: None,
                key_bytes: 0,
            },
            ledger,
        )?;
        while let Some(visit) = work.values.pop() {
            match visit {
                Visit::Enter {
                    value,
                    parent,
                    key_bytes,
                } => enter(&mut facts, &mut work, value, parent, key_bytes, ledger)?,
                Visit::Exit(index) => exit(&mut facts.values, index)?,
            }
        }
        // This is an internal consistency check against the authentic token,
        // never an authorization based on an untrusted claimed amount.
        if facts.values[0].bytes != value.exclusive_bytes()
            || facts.values[0].frontier != value.frontier()
        {
            return Err(JsonDecodeError::CapacityProfile);
        }
        drop(work);
        facts.allocation.merge(fixed)?;
        Ok(Shared::new(Self {
            facts: facts.values,
            _allocation: facts.allocation,
        }))
    }
}
/// Records one node's own bytes, links it to its parent and schedules its children.
fn enter<'a>(
    facts: &mut Metered<Fact>,
    work: &mut Metered<Visit<'a>>,
    value: &'a Value,
    parent: Option<usize>,
    key_bytes: usize,
    ledger: &DecodeLedger,
) -> Result<(), JsonDecodeError> {
    let index = facts.values.len();
    let mut bytes = match value {
        Value::String(text) => text.capacity(),
        Value::Array(items) => product(items.capacity(), size_of::<Value>())?,
        Value::Object(items) => object_layout::<Value>(items.len())?,
        _ => 0,
    };
    if let Value::Object(items) = value {
        for key in items.keys() {
            bytes = bytes
                .checked_add(key.capacity())
                .ok_or(JsonDecodeError::Arithmetic)?;
        }
    }
    facts.push(
        Fact {
            bytes,
            frontier: 1,
            key_bytes,
            first: None,
            next: None,
            last: None,
        },
        ledger,
    )?;
    if let Some(parent) = parent {
        if let Some(last) = facts.values[parent].last {
            facts.values[last].next = Some(index);
        } else {
            facts.values[parent].first = Some(index);
        }
        facts.values[parent].last = Some(index);
    }
    work.push(Visit::Exit(index), ledger)?;
    let child = |value, key_bytes| Visit::Enter {
        value,
        parent: Some(index),
        key_bytes,
    };
    match value {
        Value::Array(items) => {
            for item in items.iter().rev() {
                work.push(child(item, 0), ledger)?;
            }
        }
        Value::Object(items) => {
            for (key, item) in items.iter().rev() {
                work.push(child(item, key.capacity()), ledger)?;
            }
        }
        _ => {}
    }
    Ok(())
}
/// Folds completed children into their parent's total bytes and frontier.
fn exit(facts: &mut [Fact], index: usize) -> Result<(), JsonDecodeError> {
    let mut child = facts[index].first;
    let mut count = 0usize;
    let mut frontier = 1usize;
    let mut bytes = facts[index].bytes;
    while let Some(next) = child {
        let fact = &facts[next];
        bytes = bytes
            .checked_add(fact.bytes)
            .ok_or(JsonDecodeError::Arithmetic)?;
        frontier = frontier.max(
            fact.frontier
                .checked_add(count)
                .ok_or(JsonDecodeError::Arithmetic)?,
        );
        count = count.checked_add(1).ok_or(JsonDecodeError::Arithmetic)?;
        child = fact.next;
    }
    facts[index].bytes = bytes;
    facts[index].frontier = frontier.max(count);
    Ok(())
}
