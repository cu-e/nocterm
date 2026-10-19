//! Closed n/t/f/number leaves use the original pinned stock scalar parser.
use super::{Atom, Scalar};
use crate::loading::literal_reader::{LiteralReadError, input::Input};
use nocterm_ai::model::{failure::NativeErrorSlot, json::deferred::DecodeLedger};
use serde::Deserialize as _;
use std::io::Read;

pub(super) fn read<R: Read>(
    input: &mut Input<R>,
    ledger: &DecodeLedger,
) -> Result<Scalar, LiteralReadError> {
    // No float_roundtrip/arbitrary_precision in the selected dependency feature
    // set. The original scalar dispatcher reaches no string/Message constructor;
    // stock finite numeric parsing uses no scratch and has one ErrorImpl path.
    let slot = NativeErrorSlot::<serde_json::Error>::prepare(
        ledger,
        serde_json::Error::allocation_bytes(),
    )
    .map_err(LiteralReadError::Admission)?;
    let mut decoder = serde_json::Deserializer::from_reader(Atom {
        input: &mut *input,
        ended: false,
    });
    let result = Scalar::deserialize(&mut decoder).and_then(|scalar| {
        decoder.end()?;
        Ok(scalar)
    });
    drop(decoder);
    // A genuine retained source fault wins after all parser locals retire.
    // Known native marker owners are checked by the surrounding held replay.
    input.check()?;
    result.map_err(|error| LiteralReadError::ManagedJson(slot.capture(error)))
}

#[cfg(test)]
mod tests;
