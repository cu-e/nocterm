//! Monotonic retained v2 JSON-field construction; no global128 semantic limit.
//! Not a legacy ignored/defaulted validation path or a complete history loader.
pub(in crate::loading) mod input;
mod planned;
#[cfg(test)]
pub(super) use planned::decode_authenticated_json;
pub(super) use planned::decode_authenticated_native;
pub(in crate::loading) mod preflight;
pub(super) use preflight::original_source;
pub(super) use preflight::preflight;
#[cfg(test)]
mod exact;
pub(in crate::loading) mod scalar;
#[cfg(test)]
pub(super) use exact::decode_authenticated_text;
#[cfg(test)]
mod tests;
use super::string_reader::{JsonStringReader, StringReadError};
#[cfg(test)]
use nocterm_ai::model::json::deferred::DecodedJson;
use nocterm_ai::model::json::deferred::{
    DecodeLedger, JsonDecodeError,
    literal::{
        LiteralBuilder, LiteralSyntax, LiteralText, LiteralTextFact, LiteralTextFacts, StringRole,
    },
};
use nocterm_core::SharedError;
use std::io::{self, Read};
#[derive(Debug, thiserror::Error)]
pub(super) enum LiteralReadError {
    #[error("Original saved source exceeds the pinned JSON parser recursion budget")]
    SourceDepth,
    #[error("Literal version-two preflight failed: {0}")]
    Preflight(#[from] nocterm_ai::history::PreflightFailure<JsonDecodeError>),
    #[error("Retained JSON input exceeds its encoded bound")]
    EncodedLimit,
    #[error("Retained JSON source failed: {0}")]
    Source(#[source] SharedError<io::Error>),
    #[error("Retained JSON string failed: {0}")]
    String(#[from] StringReadError),
    #[error("Retained JSON scalar failed: {0}")]
    ManagedJson(#[source] nocterm_ai::model::failure::ManagedNativeFailure),
    #[error("Retained JSON parser admission failed: {0}")]
    Admission(#[source] nocterm_ai::model::json::deferred::NativeAllocationError),
    #[error("Retained JSON source has an original guarded failure")]
    ManagedSource,
    #[error("Retained JSON controlled construction failed: {0}")]
    Build(#[from] JsonDecodeError),
    #[error("Retained JSON has an invalid token")]
    Token,
    #[error("Retained JSON lookahead was not consumed")]
    Lookahead,
}
/// Caller must authenticate the exact consumed span before adopting the result,
/// and reserve already-held native/sibling/plan/SDK state in this same ledger.
/// Current source drives every dynamic frame/text capacity reservation; a former
/// digest/length estimate never authorizes current allocations. Native fixed state
/// and final whole-loader peak proof remain separately incomplete.
#[cfg(test)]
pub(super) fn decode_retained<R: Read>(
    source: R,
    ledger: &DecodeLedger,
) -> Result<DecodedJson, LiteralReadError> {
    let mut input = input::Input::new(source);
    let result = decode_current(&mut input, ledger, None, None);
    input.check()?;
    result
}
#[cfg(test)]
fn decode_current<R: Read>(
    input: &mut input::Input<R>,
    ledger: &DecodeLedger,
    facts: Option<&LiteralTextFacts>,
    cleanup_frontier: Option<usize>,
) -> Result<DecodedJson, LiteralReadError> {
    Ok(build_current(input, ledger, facts, cleanup_frontier)?.finish()?)
}
fn build_current<R: Read>(
    input: &mut input::Input<R>,
    ledger: &DecodeLedger,
    facts: Option<&LiteralTextFacts>,
    cleanup_frontier: Option<usize>,
) -> Result<LiteralBuilder, LiteralReadError> {
    let mut next_fact = 0usize;
    let mut syntax = LiteralSyntax::new(ledger)?;
    let mut builder = match cleanup_frontier {
        Some(frontier) => LiteralBuilder::with_cleanup_frontier(ledger, frontier)?,
        None => LiteralBuilder::new(ledger)?,
    };
    while let Some(byte) = input.nonwhite()? {
        match byte {
            b'[' => {
                syntax.start_array()?;
                builder.start_array()?;
            }
            b'{' => {
                syntax.start_object()?;
                builder.start_object()?;
            }
            b']' => {
                syntax.end_array()?;
                builder.end_container()?;
            }
            b'}' => {
                syntax.end_object()?;
                builder.end_container()?;
            }
            b',' => syntax.comma()?,
            b':' => syntax.colon()?,
            b'"' => {
                let role = syntax.string()?;
                let fact = facts
                    .map(|facts| facts.get(next_fact).ok_or(JsonDecodeError::TextPlan))
                    .transpose()?;
                if let Some(fact) = fact
                    && (fact.start() != input.offset() - 1 || fact.role() != role)
                {
                    return Err(JsonDecodeError::TextPlan.into());
                }
                input.unread(byte)?;
                let text = read_text(input, ledger, &builder, fact)?;
                next_fact += 1;
                match role {
                    StringRole::Key => builder.key(text)?,
                    StringRole::Value => builder.string(text)?,
                }
            }
            b'n' | b't' | b'f' | b'-' | b'0'..=b'9' => {
                syntax.scalar()?;
                input.unread(byte)?;
                match scalar::read(input, ledger)? {
                    scalar::Scalar::Null => builder.null()?,
                    scalar::Scalar::Boolean(value) => builder.boolean(value)?,
                    scalar::Scalar::Number(value) => builder.number(value)?,
                }
            }
            _ => return Err(LiteralReadError::Token),
        }
    }
    syntax.finish()?;
    if facts.is_some_and(|facts| facts.len() != next_fact) {
        return Err(JsonDecodeError::TextPlan.into());
    }
    Ok(builder)
}
/// One string, built exactly to its preflight fact when the source has one.
fn read_text<R: Read>(
    input: &mut input::Input<R>,
    ledger: &DecodeLedger,
    builder: &LiteralBuilder,
    fact: Option<LiteralTextFact>,
) -> Result<LiteralText, LiteralReadError> {
    let mut cursor = JsonStringReader::new_managed(
        &mut *input,
        nocterm_ai::history::MAX_FILE_BYTES as usize,
        ledger,
    )?;
    let Some(fact) = fact else {
        let mut text = builder.text()?;
        while let Some(fragment) = cursor.next_fragment()? {
            text.append(fragment)?;
        }
        return Ok(text);
    };
    let mut text = builder.exact_text(fact.decoded_bytes())?;
    while let Some(fragment) = cursor.next_fragment()? {
        text.append(fragment)?;
    }
    drop(cursor);
    if input.offset() != fact.end() {
        return Err(JsonDecodeError::TextPlan.into());
    }
    Ok(text.finish()?)
}
