//! Private transient JSON nodes; reserved raw strings have no domain wire field.
use super::{DecodeLedger, JsonDecodeError, chunks::OwnedChunks, ledger::Reservation};
use indexmap::IndexMap;
use serde_json::{Map, Value};
use std::mem::size_of;
#[derive(Debug)]
pub(super) enum Kind {
    Literal(Value),
    Array(Vec<Node>),
    Object(IndexMap<String, Node>),
    Raw(String),
}
#[derive(Debug)]
pub(super) struct Node {
    pub(super) kind: Kind,
    pub(super) allocation: Reservation,
    pub(super) layout: usize,
}
pub(super) struct Built {
    pub(super) value: Value,
    pub(super) allocation: Reservation,
}
pub(super) fn product(left: usize, right: usize) -> Result<usize, JsonDecodeError> {
    left.checked_mul(right).ok_or(JsonDecodeError::Arithmetic)
}
pub(super) fn sum(left: usize, right: usize) -> Result<usize, JsonDecodeError> {
    left.checked_add(right).ok_or(JsonDecodeError::Arithmetic)
}
pub(super) fn object_layout<T>(entries: usize) -> Result<usize, JsonDecodeError> {
    if entries == 0 {
        return Ok(0);
    }
    let buckets = match entries {
        1..=3 => 4,
        4..=7 => 8,
        8..=14 => 16,
        _ => product(entries, 8)?
            .checked_div(7)
            .and_then(usize::checked_next_power_of_two)
            .ok_or(JsonDecodeError::Arithmetic)?,
    };
    let bucket = sum(
        size_of::<usize>(),
        sum(size_of::<String>(), size_of::<T>())?,
    )?;
    sum(
        product(entries, bucket)?,
        sum(product(buckets, size_of::<usize>() + 1)?, 16)?,
    )
}
pub(super) fn next_capacity(current: usize) -> Result<usize, JsonDecodeError> {
    if current == 0 {
        Ok(4)
    } else {
        product(current, 2)
    }
}
impl Node {
    pub(super) fn build(mut self, ledger: &DecodeLedger) -> Result<Built, JsonDecodeError> {
        loop {
            let Self {
                kind,
                allocation,
                layout,
            } = self;
            return match kind {
                Kind::Literal(value) => Ok(Built { value, allocation }),
                Kind::Array(nodes) => {
                    let bytes = product(nodes.len(), size_of::<Value>())?;
                    let mut destination = ledger.reserve(bytes)?;
                    let mut values = Vec::new();
                    values.try_reserve_exact(nodes.len())?;
                    if values.capacity() != nodes.len() {
                        return Err(JsonDecodeError::CapacityProfile);
                    }
                    for node in nodes {
                        let child = node.build(ledger)?;
                        destination.merge(child.allocation)?;
                        values.push(child.value);
                    }
                    drop(allocation);
                    Ok(Built {
                        value: Value::Array(values),
                        allocation: destination,
                    })
                }
                Kind::Object(nodes) => {
                    let bytes = object_layout::<Value>(nodes.len())?;
                    let mut destination = ledger.reserve(bytes)?;
                    // Only key storage moves; the source map's dense/hash backing retires.
                    let mut source = allocation;
                    destination.merge(source.split(source.bytes() - layout)?)?;
                    let mut values = Map::with_capacity(nodes.len());
                    for (key, node) in nodes {
                        let child = node.build(ledger)?;
                        destination.merge(child.allocation)?;
                        values.insert(key, child.value);
                    }
                    drop(source);
                    Ok(Built {
                        value: Value::Object(values),
                        allocation: destination,
                    })
                }
                Kind::Raw(input) => {
                    let reader = OwnedChunks::from_reserved(input, allocation, ledger)?;
                    let mut decoder = serde_json::Deserializer::from_reader(reader);
                    let parsed = super::DeferredJson::from_deserializer(&mut decoder, ledger)?;
                    decoder.end()?;
                    // No parent IoRead scratch or raw source remains during expansion.
                    drop(decoder);
                    parsed.pending.validate(ledger)?;
                    self = parsed.root;
                    continue;
                }
            };
        }
    }
}
