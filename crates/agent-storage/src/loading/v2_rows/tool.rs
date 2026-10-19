//! Partial genuine tool-row construction; unsupported compounds stay explicit.
use super::{
    DecodeLedger, DecodedRow, JsonDecodeError, JsonStringReader, LiteralText, StringReadError,
    names,
};
use crate::loading::{
    held_file::{HeldFile, HeldReadError, plan::AuthenticatedPlan},
    json_fields::Key,
    literal_reader::{LiteralReadError, decode_authenticated_json, decode_authenticated_text},
    v2_plan::{PlanError, Scope, V2Plan},
};
use nocterm_ai::{
    history::ToolRowHeader,
    model::{
        json::deferred::DecodedJson,
        tools::{ToolKind, ToolStatus},
    },
};
use nocterm_core::SharedError;
use serde::Deserialize;
use std::io::{self, Read};
#[derive(Debug, thiserror::Error)]
pub(in crate::loading) enum ToolReadError {
    #[error("Version-two tool misses required field {0:?}")]
    Missing(Key),
    #[error("Version-two tool contains unsupported compound field {0:?}")]
    Incomplete(Key),
    #[error("Version-two tool field {0:?} has an invalid value")]
    Invalid(Key),
    #[error("Version-two tool source failed: {0}")]
    Source(#[source] SharedError<io::Error>),
    #[error("Version-two tool replay failed: {0}")]
    Held(#[from] HeldReadError),
    #[error("Version-two tool shape failed: {0}")]
    Plan(#[from] PlanError),
    #[error("Version-two tool string failed: {0}")]
    String(#[from] StringReadError),
    #[error("Version-two tool controlled allocation failed: {0}")]
    Build(#[from] JsonDecodeError),
    #[error("Version-two tool JSON failed: {0}")]
    Literal(#[from] LiteralReadError),
    #[error("Version-two tool scalar failed: {0}")]
    Json(#[source] SharedError<serde_json::Error>),
}
fn span(
    plan: &AuthenticatedPlan<V2Plan>,
    key: Key,
) -> Result<crate::loading::held_file::plan::NativeSpan, ToolReadError> {
    Ok(plan
        .payload()
        .field(key)
        .ok_or(ToolReadError::Missing(key))?
        .native_span())
}
pub(super) fn read(
    plan: &AuthenticatedPlan<V2Plan>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> Result<DecodedRow, ToolReadError> {
    // Never default/drop actual nonempty native fields merely to enter this seam.
    for key in [Key::Output, Key::Locations] {
        let selected = span(plan, key)?;
        plan.replay(held, |_| selected, |reader| expect_empty(reader, key))??;
    }
    if let Some(field) = plan.payload().field(Key::Extensions) {
        let selected = field.native_span();
        plan.replay(
            held,
            |_| selected,
            |reader| expect_none(reader, Key::Extensions),
        )??;
    }
    let display = display::read_optional(plan, held, ledger)?;
    let id = read_text(plan, held, Key::Id, ledger)?;
    let title = read_text(plan, held, Key::Title, ledger)?;
    let kind: ToolKind = read_enum(plan, held, Key::Kind)?;
    let status: ToolStatus = read_enum(plan, held, Key::Status)?;
    let mut header = ToolRowHeader::new(id, title, kind, status);
    if let Some(field) = plan.payload().field(Key::Name) {
        let selected = field.native_span();
        let name = plan.replay(held, |_| selected, |reader| optional_text(reader, ledger))??;
        if let Some(name) = name {
            header = header.with_name(name);
        }
    }
    let arguments = presence(plan, held, Key::Arguments, ledger)?;
    let result = presence(plan, held, Key::Result, ledger)?;
    Ok(DecodedRow::tool_with_display(
        header, arguments, result, display, ledger,
    )?)
}
fn text(reader: &mut dyn Read, ledger: &DecodeLedger) -> Result<LiteralText, ToolReadError> {
    let mut cursor = JsonStringReader::new(reader, nocterm_ai::history::MAX_FILE_BYTES as usize)?;
    let mut text = LiteralText::new(ledger)?;
    while let Some(fragment) = cursor.next_fragment()? {
        text.append(fragment)?;
    }
    Ok(text)
}
fn read_text(
    plan: &AuthenticatedPlan<V2Plan>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
) -> Result<LiteralText, ToolReadError> {
    let field = plan
        .payload()
        .field(key)
        .ok_or(ToolReadError::Missing(key))?;
    Ok(decode_authenticated_text(
        plan,
        held,
        |_| field.native_span(),
        ledger,
    )??)
}
fn read_enum<T: for<'de> Deserialize<'de>>(
    plan: &AuthenticatedPlan<V2Plan>,
    held: &mut HeldFile,
    key: Key,
) -> Result<T, ToolReadError> {
    let selected = span(plan, key)?;
    plan.replay(
        held,
        |_| selected,
        |reader| {
            let name = names::read(reader)?.ok_or(ToolReadError::Invalid(key))?;
            T::deserialize(serde::de::value::StrDeserializer::<serde_json::Error>::new(
                name.as_str()?,
            ))
            .map_err(|error| ToolReadError::Json(SharedError::from_error(error)))
        },
    )?
}
fn nonwhite(reader: &mut dyn Read) -> Result<Option<u8>, ToolReadError> {
    let mut byte = [0];
    loop {
        match reader.read(&mut byte) {
            Ok(0) => return Ok(None),
            Ok(_) => match byte[0] {
                b' ' | b'\n' | b'\r' | b'\t' => {}
                byte => return Ok(Some(byte)),
            },
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(ToolReadError::Source(SharedError::from_error(error))),
        }
    }
}
fn expect_empty(reader: &mut dyn Read, key: Key) -> Result<(), ToolReadError> {
    if nonwhite(reader)? != Some(b'[')
        || nonwhite(reader)? != Some(b']')
        || nonwhite(reader)?.is_some()
    {
        return Err(ToolReadError::Incomplete(key));
    }
    Ok(())
}
fn expect_none(reader: &mut dyn Read, key: Key) -> Result<(), ToolReadError> {
    if serde_json::from_reader::<_, ()>(reader).is_err() {
        return Err(ToolReadError::Incomplete(key));
    }
    Ok(())
}
fn optional_text(
    reader: &mut dyn Read,
    ledger: &DecodeLedger,
) -> Result<Option<LiteralText>, ToolReadError> {
    let first = nonwhite(reader)?.ok_or(ToolReadError::Invalid(Key::Name))?;
    let mut source = io::Cursor::new([first]).chain(reader);
    if first == b'n' {
        serde_json::from_reader::<_, ()>(&mut source)
            .map_err(|error| ToolReadError::Json(SharedError::from_error(error)))?;
        Ok(None)
    } else {
        Ok(Some(text(&mut source, ledger)?))
    }
}
fn presence(
    parent: &AuthenticatedPlan<V2Plan>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
) -> Result<Option<DecodedJson>, ToolReadError> {
    let field = parent
        .payload()
        .field(key)
        .ok_or(ToolReadError::Missing(key))?;
    let selected = field.native_span();
    let start = field.value().start();
    let plan = parent
        .derive(
            held,
            |_| selected,
            |reader| V2Plan::scan(reader, start, Scope::JsonPresence),
        )?
        .transpose()?;
    let selected = span(&plan, Key::Presence)?;
    let tag = plan.replay(
        held,
        |_| selected,
        |reader| -> Result<_, ToolReadError> {
            names::read(reader)?.ok_or(ToolReadError::Invalid(key))
        },
    )??;
    match tag.as_str()? {
        "absent" => {
            if let Some(field) = plan.payload().field(Key::Value) {
                let selected = field.native_span();
                plan.replay(held, |_| selected, |reader| expect_none(reader, key))??;
            }
            Ok(None)
        }
        "supplied" => {
            let field = plan
                .payload()
                .field(Key::Value)
                .ok_or(ToolReadError::Missing(Key::Value))?;
            Ok(Some(decode_authenticated_json(
                &plan,
                held,
                |_| field.native_span(),
                ledger,
            )??))
        }
        _ => Err(ToolReadError::Invalid(key)),
    }
}

#[cfg(test)]
mod tests;

mod display;
