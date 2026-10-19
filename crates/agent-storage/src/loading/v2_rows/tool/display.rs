//! Complete ClientDisplay fields through authenticated contextual v2 spans.
use super::*;
use nocterm_ai::{
    history::{DecodedDisplay, DecodedDisplayOutcome},
    model::controls::Presence,
};
#[cfg(test)]
mod tests;
fn selected_plan(
    parent: &AuthenticatedPlan<V2Plan>,
    held: &mut HeldFile,
    key: Key,
    scope: Scope,
) -> Result<AuthenticatedPlan<V2Plan>, ToolReadError> {
    let field = parent
        .payload()
        .field(key)
        .ok_or(ToolReadError::Missing(key))?;
    let start = field.value().start();
    let selected = field.native_span();
    Ok(parent
        .derive(
            held,
            |_| selected,
            |reader| V2Plan::scan(reader, start, scope),
        )?
        .transpose()?)
}
fn first(
    parent: &AuthenticatedPlan<V2Plan>,
    held: &mut HeldFile,
    key: Key,
) -> Result<Option<u8>, ToolReadError> {
    let selected = span(parent, key)?;
    parent.replay(held, |_| selected, nonwhite)?
}
fn unit(
    parent: &AuthenticatedPlan<V2Plan>,
    held: &mut HeldFile,
    key: Key,
) -> Result<(), ToolReadError> {
    scalar::<()>(parent, held, key)
}
fn scalar<T: for<'de> Deserialize<'de>>(
    parent: &AuthenticatedPlan<V2Plan>,
    held: &mut HeldFile,
    key: Key,
) -> Result<T, ToolReadError> {
    let selected = span(parent, key)?;
    parent
        .replay(held, |_| selected, |reader| serde_json::from_reader(reader))?
        .map_err(|error| ToolReadError::Json(SharedError::from_error(error)))
}
fn json(
    parent: &AuthenticatedPlan<V2Plan>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
) -> Result<DecodedJson, ToolReadError> {
    let field = parent
        .payload()
        .field(key)
        .ok_or(ToolReadError::Missing(key))?;
    Ok(decode_authenticated_json(
        parent,
        held,
        |_| field.native_span(),
        ledger,
    )??)
}
pub(super) fn read_optional(
    parent: &AuthenticatedPlan<V2Plan>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> Result<Option<DecodedDisplay>, ToolReadError> {
    match first(parent, held, Key::ClientDisplay)? {
        Some(b'n') => {
            unit(parent, held, Key::ClientDisplay)?;
            Ok(None)
        }
        Some(b'{' | b'[') => {
            let plan = selected_plan(parent, held, Key::ClientDisplay, Scope::Display)?;
            Ok(Some(read(&plan, held, ledger)?))
        }
        _ => Err(ToolReadError::Invalid(Key::ClientDisplay)),
    }
}
fn read(
    plan: &AuthenticatedPlan<V2Plan>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> Result<DecodedDisplay, ToolReadError> {
    let version = scalar::<u8>(plan, held, Key::Version)?;
    let tool = read_text(plan, held, Key::Tool, ledger)?;
    let arguments = json(plan, held, Key::Arguments, ledger)?;
    let destination = control(plan, held, Key::Destination, |plan, held| {
        read_text(plan, held, Key::Value, ledger)
    })?;
    let redacted = control(plan, held, Key::Redacted, |plan, held| {
        scalar::<bool>(plan, held, Key::Value)
    })?;
    let outcome = control(plan, held, Key::Outcome, |plan, held| {
        read_outcome(plan, held, ledger)
    })?;
    Ok(DecodedDisplay::new(
        version,
        tool,
        arguments,
        destination,
        redacted,
        outcome,
        ledger,
    )?)
}
fn control<T>(
    parent: &AuthenticatedPlan<V2Plan>,
    held: &mut HeldFile,
    key: Key,
    value: impl FnOnce(&AuthenticatedPlan<V2Plan>, &mut HeldFile) -> Result<T, ToolReadError>,
) -> Result<Presence<T>, ToolReadError> {
    let plan = selected_plan(parent, held, key, Scope::JsonPresence)?;
    let selected = span(&plan, Key::Presence)?;
    let tag = plan
        .replay(held, |_| selected, |reader| names::read(reader))??
        .ok_or(ToolReadError::Invalid(key))?;
    match tag.as_str()? {
        "absent" | "null" => {
            if plan.payload().field(Key::Value).is_some() {
                unit(&plan, held, Key::Value)?;
            }
            Ok(if tag.as_str()? == "absent" {
                Presence::Absent
            } else {
                Presence::Null
            })
        }
        "value" => {
            span(&plan, Key::Value)?;
            Ok(Presence::Value(value(&plan, held)?))
        }
        _ => Err(ToolReadError::Invalid(key)),
    }
}
fn read_outcome(
    parent: &AuthenticatedPlan<V2Plan>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> Result<DecodedDisplayOutcome, ToolReadError> {
    match first(parent, held, Key::Value)? {
        Some(b'"') => {
            let selected = span(parent, Key::Value)?;
            let tag = parent
                .replay(held, |_| selected, |reader| names::read(reader))??
                .ok_or(ToolReadError::Invalid(Key::Outcome))?;
            if tag.as_str()? == "Pending" {
                Ok(DecodedDisplayOutcome::Pending)
            } else {
                Err(ToolReadError::Invalid(Key::Outcome))
            }
        }
        Some(b'{') => {
            let plan = selected_plan(parent, held, Key::Value, Scope::DisplayOutcome)?;
            if plan.payload().field(Key::OutcomePending).is_some() {
                unit(&plan, held, Key::OutcomePending)?;
                Ok(DecodedDisplayOutcome::Pending)
            } else if plan.payload().field(Key::OutcomeOk).is_some() {
                Ok(DecodedDisplayOutcome::Succeeded(json(
                    &plan,
                    held,
                    Key::OutcomeOk,
                    ledger,
                )?))
            } else if plan.payload().field(Key::OutcomeErr).is_some() {
                Ok(DecodedDisplayOutcome::Failed(read_text(
                    &plan,
                    held,
                    Key::OutcomeErr,
                    ledger,
                )?))
            } else {
                Err(ToolReadError::Invalid(Key::Outcome))
            }
        }
        _ => Err(ToolReadError::Invalid(Key::Outcome)),
    }
}
