//! Shared native serde field diagnostics for the two stable flat token grammars.
use serde::{
    Deserialize,
    de::{self, SeqAccess},
};

pub(super) fn required<T, E: de::Error>(value: Option<T>, name: &'static str) -> Result<T, E> {
    value.ok_or_else(|| E::missing_field(name))
}

pub(super) fn next<'de, T: Deserialize<'de>, A: SeqAccess<'de>>(
    sequence: &mut A,
    index: usize,
    expected: &dyn de::Expected,
) -> Result<T, A::Error> {
    sequence
        .next_element()?
        .ok_or_else(|| de::Error::invalid_length(index, expected))
}

/// Serde's struct-field grammar, including numeric and borrowed byte identifiers.
pub(super) struct NameSeed(&'static [&'static str]);

impl NameSeed {
    pub(super) fn new(names: &'static [&'static str]) -> Self {
        Self(names)
    }
}

impl<'de> serde::de::DeserializeSeed<'de> for NameSeed {
    type Value = &'static str;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_identifier(self)
    }
}

impl<'de> serde::de::Visitor<'de> for NameSeed {
    type Value = &'static str;
    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("field identifier")
    }
    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        self.0
            .iter()
            .copied()
            .find(|name| *name == value)
            .ok_or_else(|| E::unknown_field(value, self.0))
    }
    fn visit_bytes<E: de::Error>(self, value: &[u8]) -> Result<Self::Value, E> {
        self.visit_str(&String::from_utf8_lossy(value))
    }
    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        usize::try_from(value)
            .ok()
            .and_then(|index| self.0.get(index))
            .copied()
            .ok_or_else(|| {
                E::invalid_value(
                    de::Unexpected::Unsigned(value),
                    &FieldIndexRange(self.0.len()),
                )
            })
    }
}

struct FieldIndexRange(usize);
impl de::Expected for FieldIndexRange {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "field index 0 <= i < {}", self.0)
    }
}
