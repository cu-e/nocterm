//! Raw-aware syntax validation constructs no ordinary JSON tree.
use super::{
    DecodeLedger, JsonDecodeError,
    pending::Pending,
    visitor::{Text, guard},
};
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use std::fmt;
pub(super) struct Validate<'a> {
    pub(super) ledger: &'a DecodeLedger,
    pub(super) pending: &'a mut Pending,
    pub(super) failure: &'a mut Option<JsonDecodeError>,
}
impl<'de> DeserializeSeed<'de> for Validate<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_any(self)
    }
}
struct Key;
impl<'de> DeserializeSeed<'de> for Key {
    type Value = bool;
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<bool, D::Error> {
        decoder.deserialize_str(self)
    }
}
impl<'de> Visitor<'de> for Key {
    type Value = bool;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("JSON object key")
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<bool, E> {
        Ok(value == "$serde_json::private::RawValue")
    }
    fn visit_string<E: serde::de::Error>(self, value: String) -> Result<bool, E> {
        self.visit_str(&value)
    }
}
impl<'de> Visitor<'de> for Validate<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("JSON")
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
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
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_string<E: serde::de::Error>(self, _: String) -> Result<(), E> {
        Ok(())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<(), A::Error> {
        while sequence
            .next_element_seed(Validate {
                ledger: self.ledger,
                pending: self.pending,
                failure: self.failure,
            })?
            .is_some()
        {}
        Ok(())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let Some(raw) = map.next_key_seed(Key)? else {
            return Ok(());
        };
        if raw {
            let (text, allocation) = map.next_value_seed(Text {
                ledger: self.ledger,
                failure: self.failure,
            })?;
            return guard(
                self.pending.push_raw(text, allocation, self.ledger),
                self.failure,
            );
        }
        map.next_value_seed(Validate {
            ledger: self.ledger,
            pending: self.pending,
            failure: self.failure,
        })?;
        while map.next_key::<IgnoredAny>()?.is_some() {
            map.next_value_seed(Validate {
                ledger: self.ledger,
                pending: self.pending,
                failure: self.failure,
            })?;
        }
        Ok(())
    }
}
