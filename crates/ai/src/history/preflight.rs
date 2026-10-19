//! CPU-only structural preflight accepts a supplied serde decoder, never native I/O.
use super::HistoryCodecError;
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
pub(super) const LIMIT: usize = 64 * 1024 * 1024;
/// Exact existing metric plus the facts needed for a separate allocation proof.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreflightBounds {
    pub structural_bytes: usize,
    pub nodes: usize,
    pub decoded_text_bytes: usize,
    pub maximum_text_bytes: usize,
}
#[derive(Debug, thiserror::Error)]
pub enum PreflightFailure<E: std::error::Error + 'static> {
    #[error("Saved document exceeds its structural memory limit")]
    StructuralLimit,
    #[error("Saved document preflight arithmetic overflow")]
    ArithmeticOverflow,
    #[error("Saved document JSON preflight failed: {0}")]
    Json(#[source] E),
}
#[derive(Clone, Copy)]
enum BudgetFault {
    Limit,
    Overflow,
}
struct Budget {
    bounds: PreflightBounds,
    failed: Option<BudgetFault>,
}
impl Budget {
    fn overflow<E: serde::de::Error>(&mut self) -> E {
        self.failed = Some(BudgetFault::Overflow);
        E::custom("saved document preflight arithmetic overflow")
    }
    fn add<E: serde::de::Error>(&mut self, bytes: usize) -> Result<(), E> {
        self.bounds.structural_bytes = self
            .bounds
            .structural_bytes
            .checked_add(bytes)
            .ok_or_else(|| self.overflow())?;
        if self.bounds.structural_bytes > LIMIT {
            self.failed = Some(BudgetFault::Limit);
            Err(E::custom(
                "saved document exceeds its structural memory limit",
            ))
        } else {
            Ok(())
        }
    }
    fn node<E: serde::de::Error>(&mut self) -> Result<(), E> {
        self.bounds.nodes = self
            .bounds
            .nodes
            .checked_add(1)
            .ok_or_else(|| self.overflow())?;
        self.add(512)
    }
    fn text<E: serde::de::Error>(&mut self, value: &str) -> Result<(), E> {
        self.bounds.decoded_text_bytes = self
            .bounds
            .decoded_text_bytes
            .checked_add(value.len())
            .ok_or_else(|| self.overflow())?;
        self.bounds.maximum_text_bytes = self.bounds.maximum_text_bytes.max(value.len());
        let bytes = value.len().checked_mul(2).ok_or_else(|| self.overflow())?;
        self.add(bytes)
    }
}
struct Node<'a>(&'a mut Budget);
impl<'de> DeserializeSeed<'de> for Node<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        self.0.node::<D::Error>()?;
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
        self.0.text(value)
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
/// The adapter creates a bounded decoder and checks its end-of-input. This seed
/// constructs no document tree and performs no read/open/seek itself.
/// These bounds are inputs to a peak proof, never proof merely by themselves.
pub fn check_decoder<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> Result<PreflightBounds, PreflightFailure<D::Error>>
where
    D::Error: std::error::Error + 'static,
{
    let mut budget = Budget {
        bounds: PreflightBounds::default(),
        failed: None,
    };
    if let Err(error) = Node(&mut budget).deserialize(decoder) {
        return Err(match budget.failed {
            Some(BudgetFault::Limit) => PreflightFailure::StructuralLimit,
            Some(BudgetFault::Overflow) => PreflightFailure::ArithmeticOverflow,
            None => PreflightFailure::Json(error),
        });
    }
    Ok(budget.bounds)
}
pub(super) fn check(bytes: &[u8]) -> Result<usize, HistoryCodecError> {
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let bounds = check_decoder(&mut decoder).map_err(|error| match error {
        PreflightFailure::StructuralLimit => HistoryCodecError::StructuralLimit,
        PreflightFailure::ArithmeticOverflow => HistoryCodecError::ArithmeticOverflow,
        PreflightFailure::Json(error) => error.into(),
    })?;
    decoder.end().map_err(HistoryCodecError::from)?;
    Ok(bounds.structural_bytes)
}
