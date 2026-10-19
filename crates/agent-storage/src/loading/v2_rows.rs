//! Final typed v2 Agent/Thought row construction; composite variants pending.
use super::{
    held_file::{HeldFile, HeldReadError, plan::AuthenticatedPlan},
    json_fields::Key,
    literal_reader::{LiteralReadError, decode_authenticated_text},
    string_reader::{JsonStringReader, StringReadError},
    v2_plan::V2Plan,
};
use nocterm_ai::{
    history::{DecodedRow, SavedRowKind},
    model::json::deferred::{DecodeLedger, JsonDecodeError, literal::LiteralText},
};
use std::io::Read;
mod names;
#[cfg(test)]
mod tests;
mod tool;
#[derive(Debug, thiserror::Error)]
pub(super) enum RowReadError {
    #[error("Version-two tool row failed: {0}")]
    Tool(#[from] tool::ToolReadError),
    #[error("Version-two row controlled text failed: {0}")]
    Literal(#[from] LiteralReadError),
    #[error("Version-two row has no kind or value field")]
    Shape,
    #[error("Version-two row kind is unknown")]
    UnknownKind,
    #[error("Version-two composite row {0:?} construction is not implemented")]
    Composite(SavedRowKind),
    #[error("Version-two row replay failed: {0}")]
    Held(#[from] HeldReadError),
    #[error("Version-two row string failed: {0}")]
    String(#[from] StringReadError),
    #[error("Version-two row allocation failed: {0}")]
    Build(#[from] JsonDecodeError),
}
/// Partial direct-row capability, intentionally not called by production loader.
/// The returned opaque owner keeps Entry and authentic reservations inseparable.
pub(super) fn read_text(
    plan: &AuthenticatedPlan<V2Plan>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> Result<DecodedRow, RowReadError> {
    let kind_span = plan
        .payload()
        .field(Key::Kind)
        .ok_or(RowReadError::Shape)?
        .native_span();
    let value = plan
        .payload()
        .field(Key::Value)
        .ok_or(RowReadError::Shape)?;
    let kind = plan.replay(held, |_| kind_span, read_kind)??;
    if !matches!(kind, SavedRowKind::Agent | SavedRowKind::Thought) {
        return Err(RowReadError::Composite(kind));
    }
    let text = decode_authenticated_text(plan, held, |_| value.native_span(), ledger)??;
    Ok(DecodedRow::text(kind, text, ledger)?)
}
fn read_kind(source: &mut dyn Read) -> Result<SavedRowKind, RowReadError> {
    let name = names::read(source)?.ok_or(RowReadError::UnknownKind)?;
    SavedRowKind::from_name(name.as_str()?).ok_or(RowReadError::UnknownKind)
}
/// Incomplete probe dispatcher: User/Content and nonempty tool compound fields
/// remain explicit errors, never a production default/skip/malformed decision.
pub(super) fn read_row(
    plan: &AuthenticatedPlan<V2Plan>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> Result<DecodedRow, RowReadError> {
    let kind_span = plan
        .payload()
        .field(Key::Kind)
        .ok_or(RowReadError::Shape)?
        .native_span();
    let kind = plan.replay(held, |_| kind_span, read_kind)??;
    match kind {
        SavedRowKind::Agent | SavedRowKind::Thought => read_text(plan, held, ledger),
        SavedRowKind::Tool => {
            let field = plan
                .payload()
                .field(Key::Value)
                .ok_or(RowReadError::Shape)?;
            let start = field.value().start();
            let span = field.native_span();
            let tool = plan
                .derive(
                    held,
                    |_| span,
                    |reader| V2Plan::scan(reader, start, super::v2_plan::Scope::Tool),
                )?
                .transpose()
                .map_err(tool::ToolReadError::from)?;
            Ok(tool::read(&tool, held, ledger)?)
        }
        other => Err(RowReadError::Composite(other)),
    }
}
