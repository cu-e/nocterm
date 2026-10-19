//! General nested JSON text planning using three authenticated linear span passes.
#[cfg(test)]
use super::decode_current;
use super::{LiteralReadError, input::Input, preflight};
use crate::loading::held_file::{
    HeldFile, HeldReadError,
    plan::{AuthenticatedPlan, NativeSpan},
};
#[cfg(test)]
use nocterm_ai::model::json::deferred::DecodedJson;
use nocterm_ai::model::json::deferred::{DecodeLedger, JsonDecodeError, literal::LiteralTextFacts};
use std::io;

/// Every key/value string, including duplicates, gets one lexical fact. All three
/// passes use the same held source and selected native span; no public length hint
/// or list of untrusted caller facts can enter this construction route.
#[cfg(test)]
pub(in crate::loading) fn decode_authenticated_json<T>(
    parent: &AuthenticatedPlan<T>,
    held: &mut HeldFile,
    select: impl Fn(&T) -> NativeSpan,
    ledger: &DecodeLedger,
) -> Result<Result<DecodedJson, LiteralReadError>, HeldReadError> {
    decode_planned(
        parent,
        held,
        select,
        ledger,
        |reader, ledger, facts, frontier| {
            let mut input = Input::new_managed_marker(crate::loading::held_file::replay_buffer::ReplayBuffer::<_, 256>::with_inline_capacity(reader));
            let result = decode_current(&mut input, ledger, Some(facts), Some(frontier));
            input.check()?;
            result
        },
    )
}
pub(in crate::loading) fn decode_authenticated_native<T>(
    parent: &AuthenticatedPlan<T>,
    held: &mut HeldFile,
    select: impl Fn(&T) -> NativeSpan,
    ledger: &DecodeLedger,
) -> Result<
    Result<nocterm_ai::model::json::deferred::native::NativeNode, LiteralReadError>,
    HeldReadError,
> {
    decode_planned(
        parent,
        held,
        select,
        ledger,
        |reader, ledger, facts, frontier| {
            let mut input = Input::new_managed_marker(crate::loading::held_file::replay_buffer::ReplayBuffer::<_, 256>::with_inline_capacity(reader));
            let result = super::build_current(&mut input, ledger, Some(facts), Some(frontier))?
                .finish_native()?;
            input.check()?;
            Ok(result)
        },
    )
}
pub(in crate::loading) fn decode_planned<T, U, E: From<LiteralReadError>>(
    parent: &AuthenticatedPlan<T>,
    held: &mut HeldFile,
    select: impl Fn(&T) -> NativeSpan,
    ledger: &DecodeLedger,
    build: impl FnOnce(&mut dyn io::Read, &DecodeLedger, &LiteralTextFacts, usize) -> Result<U, E>,
) -> Result<Result<U, E>, HeldReadError> {
    let count = parent.replay(held, &select, |reader| {
        let mut count = 0usize;
        let (_, frontier) = preflight::planned_walk(
            crate::loading::held_file::replay_buffer::ReplayBuffer::<_, 256>::with_inline_capacity(
                reader,
            ),
            ledger,
            |_, _, _, _| {
                count = count.checked_add(1).ok_or(JsonDecodeError::Arithmetic)?;
                Ok(())
            },
        )?;
        Ok::<_, LiteralReadError>((count, frontier))
    })?;
    let (count, frontier) = match count {
        Ok(count) => count,
        Err(error) => return Ok(Err(error.into())),
    };
    let mut facts = match LiteralTextFacts::new(count, ledger) {
        Ok(facts) => facts,
        Err(error) => return Ok(Err(LiteralReadError::from(error).into())),
    };
    let recorded = parent.replay(held, &select, |reader| {
        preflight::walk(
            crate::loading::held_file::replay_buffer::ReplayBuffer::<_, 256>::with_inline_capacity(
                reader,
            ),
            ledger,
            |start, end, length, role| {
                facts.record(start, end, length, role)?;
                Ok(())
            },
        )
    })?;
    if let Err(error) = recorded {
        return Ok(Err(error.into()));
    }
    let facts = match facts.finish() {
        Ok(facts) => facts,
        Err(error) => return Ok(Err(LiteralReadError::from(error).into())),
    };
    parent.replay(held, select, |reader| {
        build(reader, ledger, &facts, frontier)
    })
}
#[cfg(test)]
mod tests;
