//! One inline Unicode result; the original stock parser owns all escape rules.
use super::StringReadError;
use nocterm_ai::model::{
    failure::NativeErrorSlot,
    json::deferred::{DecodeLedger, NativeScratch},
};
use serde::{
    Deserialize,
    de::{self, Visitor},
};
use std::fmt;

pub(in crate::loading) struct DecodedUnit {
    bytes: [u8; 4],
    length: u8,
}
impl DecodedUnit {
    pub(in crate::loading) fn as_bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.length)]
    }
    pub(in crate::loading) fn into_parts(self) -> ([u8; 4], u8) {
        (self.bytes, self.length)
    }
}
impl<'de> Deserialize<'de> for DecodedUnit {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct CopyUnit;
        impl Visitor<'_> for CopyUnit {
            type Value = DecodedUnit;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("one Unicode unit")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<DecodedUnit, E> {
                if value.len() > 4 {
                    return Err(E::custom("invalid Unicode unit length"));
                }
                let mut bytes = [0; 4];
                bytes[..value.len()].copy_from_slice(value.as_bytes());
                Ok(DecodedUnit {
                    bytes,
                    length: value.len() as u8,
                })
            }
        }
        deserializer.deserialize_str(CopyUnit)
    }
}
/// Only the original cursors' closed quoted escape/control frames enter here.
/// Their decoded unit is at most four bytes: pinned Vec<u8> scratch grows to
/// exactly eight. Lexical errors own one ErrorImpl and no custom Message string.
pub(in crate::loading) fn parse(
    quoted: &[u8],
    ledger: Option<&DecodeLedger>,
) -> Result<DecodedUnit, StringReadError> {
    let Some(ledger) = ledger else {
        return serde_json::from_slice(quoted).map_err(StringReadError::json);
    };
    let scratch = NativeScratch::reserve_native(ledger, 8).map_err(StringReadError::Admission)?;
    let slot = NativeErrorSlot::<serde_json::Error>::prepare(
        ledger,
        serde_json::Error::allocation_bytes(),
    )
    .map_err(StringReadError::Admission)?;
    let mut decoder = serde_json::Deserializer::from_slice(quoted);
    let result = DecodedUnit::deserialize(&mut decoder).and_then(|unit| {
        decoder.end()?;
        Ok(unit)
    });
    // The actual parser Vec dies before its credit; an original error retains
    // its own separate prospective body credit through capture and last clone.
    drop(decoder);
    drop(scratch);
    result.map_err(|error| StringReadError::ManagedJson(slot.capture(error)))
}
