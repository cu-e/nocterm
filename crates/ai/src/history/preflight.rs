//! Structural accounting before serde allocates protocol objects and their vectors.
use super::HistoryCodecError;
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
pub(super) const LIMIT: usize = 64 * 1024 * 1024;
struct Budget {
    bytes: usize,
    exceeded: bool,
}
impl Budget {
    fn add<E: serde::de::Error>(&mut self, bytes: usize) -> Result<(), E> {
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes > LIMIT {
            self.exceeded = true;
            Err(E::custom(
                "saved document exceeds its structural memory limit",
            ))
        } else {
            Ok(())
        }
    }
}
struct Node<'a>(&'a mut Budget);
impl<'de> DeserializeSeed<'de> for Node<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        // Includes vector capacity, enum/struct storage and object/map nodes.
        self.0.add::<D::Error>(512)?;
        deserializer.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Node<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("bounded JSON")
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<(), E> {
        self.0.add(value.len().saturating_mul(2))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        while seq.next_element_seed(Node(self.0))?.is_some() {}
        Ok(())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while map.next_key_seed(Node(self.0))?.is_some() {
            map.next_value_seed(Node(self.0))?;
        }
        Ok(())
    }
}
pub(super) fn check(bytes: &[u8]) -> Result<usize, HistoryCodecError> {
    let mut budget = Budget {
        bytes: 0,
        exceeded: false,
    };
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    if let Err(error) = Node(&mut budget).deserialize(&mut decoder) {
        return Err(if budget.exceeded {
            HistoryCodecError::StructuralLimit
        } else {
            error.into()
        });
    }
    decoder.end().map_err(HistoryCodecError::from)?;
    Ok(budget.bytes)
}
