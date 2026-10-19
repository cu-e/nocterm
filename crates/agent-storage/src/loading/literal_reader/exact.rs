//! Scalar-span planning for the actual controlled text owner. No encoded-size hint.
//! General nested scalar discovery remains separate from this callable capability.
use super::{LiteralReadError, input::Input};
use crate::loading::held_file::replay_buffer::ReplayBuffer;
use crate::loading::{
    held_file::{
        HeldFile, HeldReadError,
        plan::{AuthenticatedPlan, NativeSpan},
    },
    string_reader::JsonStringReader,
};
use nocterm_ai::model::json::deferred::{
    DecodeLedger, DecodedJson, JsonDecodeError,
    literal::{LiteralBuilder, LiteralText},
};
use std::io::Read;

/// Both passes are authenticated against the same held source and exact selected
/// span. Measurement retains only a checked length; no decoded source buffer.
/// Changed current text is checked fragment-by-fragment BEFORE any append growth;
/// equal-length mutations still fail the second native span digest before adoption.
pub(in crate::loading) fn decode_authenticated_string<T>(
    plan: &AuthenticatedPlan<T>,
    held: &mut HeldFile,
    select: impl Fn(&T) -> NativeSpan,
    ledger: &DecodeLedger,
) -> Result<Result<DecodedJson, LiteralReadError>, HeldReadError> {
    Ok(
        decode_authenticated_text(plan, held, select, ledger)?.and_then(|text| {
            let mut builder = LiteralBuilder::new(ledger)?;
            builder.string(text)?;
            Ok(builder.finish()?)
        }),
    )
}
/// Same allocation capability consumed directly by genuine typed row leaves.
pub(in crate::loading) fn decode_authenticated_text<T>(
    plan: &AuthenticatedPlan<T>,
    held: &mut HeldFile,
    select: impl Fn(&T) -> NativeSpan,
    ledger: &DecodeLedger,
) -> Result<Result<LiteralText, LiteralReadError>, HeldReadError> {
    let measured = plan.replay(held, &select, |reader| measure(reader, ledger))?;
    let length = match measured {
        Ok(length) => length,
        Err(error) => return Ok(Err(error)),
    };
    plan.replay(held, select, |reader| build(reader, length, ledger))
}
fn measure(source: &mut dyn Read, ledger: &DecodeLedger) -> Result<usize, LiteralReadError> {
    let mut input = Input::new_managed_marker(ReplayBuffer::<_, 256>::with_inline_capacity(source));
    let mut length = 0usize;
    let mut cursor = cursor(&mut input, ledger)?;
    while let Some(fragment) = cursor.next_fragment()? {
        length = length
            .checked_add(fragment.len())
            .ok_or(JsonDecodeError::Arithmetic)?;
    }
    drop(cursor);
    end(&mut input)?;
    Ok(length)
}
fn build(
    source: &mut dyn Read,
    length: usize,
    ledger: &DecodeLedger,
) -> Result<LiteralText, LiteralReadError> {
    let mut input = Input::new_managed_marker(ReplayBuffer::<_, 256>::with_inline_capacity(source));
    let builder = LiteralBuilder::new(ledger)?;
    let mut text = builder.exact_text(length)?;
    let mut cursor = cursor(&mut input, ledger)?;
    while let Some(fragment) = cursor.next_fragment()? {
        text.append(fragment)?;
    }
    drop(cursor);
    end(&mut input)?;
    Ok(text.finish()?)
}
fn cursor<'a, 'b, R: Read>(
    input: &'a mut Input<R>,
    ledger: &'b DecodeLedger,
) -> Result<JsonStringReader<'b, &'a mut Input<R>>, LiteralReadError> {
    let byte = input.nonwhite()?.ok_or(LiteralReadError::Token)?;
    input.unread(byte)?;
    Ok(JsonStringReader::new_managed(
        input,
        nocterm_ai::history::MAX_FILE_BYTES as usize,
        ledger,
    )?)
}
fn end<R: Read>(input: &mut Input<R>) -> Result<(), LiteralReadError> {
    if input.nonwhite()?.is_some() {
        return Err(LiteralReadError::Token);
    }
    input.check()
}
#[cfg(test)]
mod tests;
