//! Closed stock u32/null parsers pay their source-derived temporary/error peaks.
#[cfg(test)]
mod tests;
use super::RouteError;
use nocterm_ai::model::{
    failure::NativeErrorSlot,
    json::deferred::{DecodeLedger, NativeAllocationError, NativeScratch},
};
use std::io::{self, Cursor, Read};

fn arithmetic<T>() -> Result<T, RouteError> {
    Err(RouteError::Admission(NativeAllocationError::Arithmetic))
}
fn add(left: usize, right: usize) -> Result<usize, RouteError> {
    left.checked_add(right)
        .ok_or(RouteError::Admission(NativeAllocationError::Arithmetic))
}
fn multiply(left: usize, right: usize) -> Result<usize, RouteError> {
    left.checked_mul(right)
        .ok_or(RouteError::Admission(NativeAllocationError::Arithmetic))
}
struct Profile {
    scratch: usize,
    error: usize,
}
impl Profile {
    fn version(first: Option<u8>, encoded: usize) -> Result<Self, RouteError> {
        // serde_core Unexpected + serde_json JsonUnexpected use fixed formatting
        // for every nonstring branch. zmij's pinned f64 output buffer is24 bytes.
        let unexpected = [
            "floating point `".len() + 24 + "`".len(),
            "integer `".len() + "-9223372036854775808".len() + "`".len(),
            "boolean `false`".len(),
            "null".len(),
            "sequence".len(),
            "map".len(),
        ]
        .into_iter()
        .max()
        .unwrap_or(0);
        let suffix = ", expected ".len() + "u32".len();
        let fixed = add("invalid value: ".len() + suffix, unexpected)?;
        let initial = multiply("invalid value: ".len() + ", expected ".len(), 2)?;
        let (scratch, message) = if first == Some(b'"') {
            // IoRead pushes decoded bytes into one Vec<u8>: initial8, doubling.
            // Decoded length <= the authenticated encoded length, even escapes.
            // Two final-capacity units cover actual old+new realloc overlap.
            let Some(capacity) = encoded.max(8).checked_next_power_of_two() else {
                return arithmetic();
            };
            let scratch = multiply(capacity, 2)?;
            // Unexpected::Str uses `string {:?}`. Debug escaping costs <=6
            // bytes per input UTF8 byte; quotes add2, then fixed format pieces.
            let message = add(
                "invalid type: ".len() + "string ".len() + 2 + suffix,
                multiply(encoded, 6)?,
            )?;
            (scratch, message.max(fixed))
        } else {
            // With !float_roundtrip/!arbitrary_precision, numeric spelling length
            // creates no scratch or message-size dependency. Preserve that.
            (0, fixed)
        };
        // Arguments::to_string -> alloc::fmt::format doubles literal width.
        // RawVec grows max(2*old,required,8), so capacity <=2*max(initial,len).
        // Three such capacity units cover old+new growth and boxed-str shrink.
        let capacity = multiply(message.max(initial), 2)?;
        let message_peak = multiply(capacity, 3)?;
        // fix_position moves ErrorCode into a replacement while old ErrorImpl
        // is still alive. Outer fix_position observes the already-set line.
        let bodies = multiply(serde_json::Error::allocation_bytes(), 2)?;
        Ok(Self {
            scratch,
            error: add(bodies, message_peak)?,
        })
    }
}

pub(super) fn version(
    source: &mut dyn Read,
    encoded: usize,
    ledger: &DecodeLedger,
) -> Result<u32, RouteError> {
    // ObjectFrames::next_member consumes nonwhite() before framed_value; the
    // authenticated span starts on its first nonwhitespace token byte.
    let mut first = [0];
    let count = loop {
        match source.read(&mut first) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            // This source is the original managed held span; its context owns
            // the first native cause and overrides after these locals drop.
            Err(_) => return Err(RouteError::ManagedSource),
            Ok(count) => break count,
        }
    };
    let profile = Profile::version(first[..count].first().copied(), encoded)?;
    let scratch =
        NativeScratch::reserve_native(ledger, profile.scratch).map_err(RouteError::Admission)?;
    let error = NativeErrorSlot::<serde_json::Error>::prepare(ledger, profile.error)
        .map_err(RouteError::Admission)?;
    let result = serde_json::from_reader::<_, u32>(Cursor::new(&first[..count]).chain(source));
    // from_reader has destroyed the real Deserializer and its scratch already.
    drop(scratch);
    result.map_err(|original| {
        let bytes = serde_json::Error::allocation_bytes() + original.message_allocation_bytes();
        RouteError::ManagedJson(error.capture_with_nested(original, bytes))
    })
}

pub(super) fn null(source: impl Read, ledger: &DecodeLedger) -> Result<(), RouteError> {
    // Caller has observed n and chained it back: parse_ident("ull") and end()
    // construct at most one original syntax/io ErrorImpl, no custom message.
    let error = NativeErrorSlot::<serde_json::Error>::prepare(
        ledger,
        serde_json::Error::allocation_bytes(),
    )
    .map_err(RouteError::Admission)?;
    serde_json::from_reader::<_, ()>(source)
        .map_err(|original| RouteError::ManagedJson(error.capture(original)))
}
