//! Structural accounting before serde allocates protocol objects and their vectors.
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::{fs::File, io::Read, path::Path};
pub(super) const LIMIT: usize = 64 * 1024 * 1024;
struct Budget(usize);
impl Budget {
    fn add<E: serde::de::Error>(&mut self, bytes: usize) -> Result<(), E> {
        self.0 = self.0.saturating_add(bytes);
        if self.0 > LIMIT {
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
pub(super) fn read(path: &Path) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    if file.metadata().map_err(|error| error.to_string())?.len() > super::MAX_FILE_BYTES {
        return Err("Saved chat exceeds the storage limit.".into());
    }
    let mut bytes = Vec::new();
    file.take(super::MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > super::MAX_FILE_BYTES {
        return Err("Saved chat exceeds the storage limit.".into());
    }
    Ok(bytes)
}
pub(super) fn check(bytes: &[u8]) -> Result<usize, String> {
    let mut budget = Budget(0);
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    Node(&mut budget)
        .deserialize(&mut decoder)
        .map_err(|error| error.to_string())?;
    decoder.end().map_err(|error| error.to_string())?;
    Ok(budget.0)
}
