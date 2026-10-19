//! Stock serde lexical grammar with controlled transient container construction.
use super::{DecodeLedger, JsonDecodeError, ledger::Reservation, nodes::*};
use indexmap::IndexMap;
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use std::{fmt, mem::size_of};
const RAW_KEY: &str = "$serde_json::private::RawValue";
pub(super) fn guard<T, E: serde::de::Error>(
    result: Result<T, JsonDecodeError>,
    failure: &mut Option<JsonDecodeError>,
) -> Result<T, E> {
    result.map_err(|error| {
        *failure = Some(error);
        E::custom("managed JSON construction failed")
    })
}
pub(super) struct Seed<'a> {
    pub(super) ledger: &'a DecodeLedger,
    pub(super) failure: &'a mut Option<JsonDecodeError>,
    pub(super) pending: &'a mut super::pending::Pending,
}
impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Node;
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<Node, D::Error> {
        decoder.deserialize_any(self)
    }
}
pub(super) struct Text<'a> {
    pub(super) ledger: &'a DecodeLedger,
    pub(super) failure: &'a mut Option<JsonDecodeError>,
}
impl<'de> DeserializeSeed<'de> for Text<'_> {
    type Value = (String, Reservation);
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<Self::Value, D::Error> {
        decoder.deserialize_str(self)
    }
}
impl<'de> Visitor<'de> for Text<'_> {
    type Value = (String, Reservation);
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a JSON string")
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
        let result = (|| {
            let allocation = self.ledger.reserve(value.len())?;
            let mut text = String::new();
            text.try_reserve_exact(value.len())?;
            if text.capacity() != value.len() {
                return Err(JsonDecodeError::CapacityProfile);
            }
            text.push_str(value);
            Ok((text, allocation))
        })();
        guard(result, self.failure)
    }
    fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
        // A generic caller's moved String was allocated by that caller. Its reserve
        // is accounted at adoption; native managed parser ingress is a later seam.
        let allocation = guard(self.ledger.reserve(value.capacity()), self.failure)?;
        Ok((value, allocation))
    }
}
impl Seed<'_> {
    fn literal<E: serde::de::Error>(self, value: Value) -> Result<Node, E> {
        let allocation = guard(self.ledger.reserve(0), self.failure)?;
        Ok(Node {
            kind: Kind::Literal(value),
            allocation,
            layout: 0,
        })
    }
}
impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Node;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("JSON")
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<Node, E> {
        self.literal(Value::Null)
    }
    fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Node, E> {
        self.literal(Value::Bool(value))
    }
    fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Node, E> {
        self.literal(Value::Number(value.into()))
    }
    fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Node, E> {
        self.literal(Value::Number(value.into()))
    }
    fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Node, E> {
        let value = serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number);
        self.literal(value)
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Node, E> {
        let (value, allocation) = Text {
            ledger: self.ledger,
            failure: self.failure,
        }
        .visit_str(value)?;
        let layout = allocation.bytes();
        Ok(Node {
            kind: Kind::Literal(Value::String(value)),
            allocation,
            layout,
        })
    }
    fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Node, E> {
        let (value, allocation) = Text {
            ledger: self.ledger,
            failure: self.failure,
        }
        .visit_string(value)?;
        let layout = allocation.bytes();
        Ok(Node {
            kind: Kind::Literal(Value::String(value)),
            allocation,
            layout,
        })
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Node, A::Error> {
        let mut nodes = Vec::new();
        let mut allocation = guard(self.ledger.reserve(0), self.failure)?;
        // Ignore untrusted size_hint. Each actual incoming child is already charged.
        while let Some(node) = sequence.next_element_seed(Seed {
            ledger: self.ledger,
            failure: self.failure,
            pending: self.pending,
        })? {
            if nodes.len() == nodes.capacity() {
                let capacity = guard(next_capacity(nodes.capacity()), self.failure)?;
                let bytes = guard(product(capacity, size_of::<Node>()), self.failure)?;
                // Both old and new storage remain charged across realloc.
                let next = guard(self.ledger.reserve(bytes), self.failure)?;
                guard(
                    nodes
                        .try_reserve_exact(capacity - nodes.len())
                        .map_err(Into::into),
                    self.failure,
                )?;
                if nodes.capacity() != capacity {
                    return guard(Err(JsonDecodeError::CapacityProfile), self.failure);
                }
                allocation = next;
            }
            nodes.push(node);
        }
        let layout = allocation.bytes();
        Ok(Node {
            kind: Kind::Array(nodes),
            allocation,
            layout,
        })
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Node, A::Error> {
        let Some((first, first_allocation)) = map.next_key_seed(Text {
            ledger: self.ledger,
            failure: self.failure,
        })?
        else {
            let allocation = guard(self.ledger.reserve(0), self.failure)?;
            return Ok(Node {
                kind: Kind::Object(IndexMap::new()),
                allocation,
                layout: 0,
            });
        };
        if first == RAW_KEY {
            // Match stock Value: only the first key is special, and its value must
            // deserialize_str. The enclosing stock map end check rejects extra keys.
            drop(first);
            drop(first_allocation);
            let (text, allocation) = map.next_value_seed(Text {
                ledger: self.ledger,
                failure: self.failure,
            })?;
            let layout = allocation.bytes();
            return Ok(Node {
                kind: Kind::Raw(text),
                allocation,
                layout,
            });
        }
        let mut nodes = IndexMap::new();
        let mut backing = guard(self.ledger.reserve(0), self.failure)?;
        let mut keys = guard(self.ledger.reserve(0), self.failure)?;
        let mut capacity = 0;
        let mut incoming = Some((first, first_allocation));
        while let Some((key, key_allocation)) = incoming {
            let value = map.next_value_seed(Seed {
                ledger: self.ledger,
                failure: self.failure,
                pending: self.pending,
            })?;
            if nodes.contains_key(&key) {
                // IndexMap retains the original key/order; drops the replaced child.
                if let Some(replaced) = nodes.insert(key, value) {
                    guard(self.pending.extract(replaced, self.ledger), self.failure)?;
                }
                drop(key_allocation);
            } else {
                if nodes.len() == capacity {
                    capacity = guard(next_capacity(capacity), self.failure)?;
                    let bytes = guard(object_layout::<Node>(capacity), self.failure)?;
                    let next = guard(self.ledger.reserve(bytes), self.failure)?;
                    guard(
                        nodes
                            .try_reserve_exact(capacity - nodes.len())
                            .map_err(Into::into),
                        self.failure,
                    )?;
                    if nodes.capacity() != capacity {
                        return guard(Err(JsonDecodeError::CapacityProfile), self.failure);
                    }
                    backing = next;
                }
                guard(keys.merge(key_allocation), self.failure)?;
                nodes.insert(key, value);
            }
            incoming = map.next_key_seed(Text {
                ledger: self.ledger,
                failure: self.failure,
            })?;
        }
        let layout = backing.bytes();
        guard(backing.merge(keys), self.failure)?;
        Ok(Node {
            kind: Kind::Object(nodes),
            allocation: backing,
            layout,
        })
    }
}
