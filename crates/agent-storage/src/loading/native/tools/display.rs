//! Presence and display outcome retain absent/null/value distinctions.
use super::{Error, Scope, f, s};
use crate::loading::{
    held_file::{HeldFile, plan::AuthenticatedPlan},
    json_fields::{Key, Key::*},
};
use nocterm_ai::model::{
    controls::Presence as Supplied, json::deferred::DecodeLedger, native::NativeOwned,
    shared_display::SharedDisplay, tools::DisplayOutcome,
};
pub(in crate::loading::native) fn display(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<SharedDisplay>, Error> {
    let p = f::child(plan, held, key, s::DISPLAY, ledger)?;
    let version = u8::try_from(f::unsigned(&p, held, Version, ledger)?).map_err(|_| Error::Type)?;
    let tool = f::text(&p, held, Tool, ledger)?;
    let arguments =
        nocterm_ai::model::native::content::json(f::node(&p, held, Arguments, ledger)?, ledger)?;
    let destination = presence(&p, held, Destination, ledger, false, |p, h, l| {
        f::text(p, h, Value, l)
    })?;
    let redacted = scalar_presence(&p, held, ledger)?;
    let outcome = presence(&p, held, Outcome, ledger, true, outcome)?;
    Ok(nocterm_ai::model::native::tools::shared(
        nocterm_ai::model::native::tools::display(
            version,
            tool,
            arguments,
            destination,
            redacted,
            outcome,
        )?,
    )?)
}
fn presence<T>(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
    semantic: bool,
    decode: impl FnOnce(
        &AuthenticatedPlan<Scope>,
        &mut HeldFile,
        &DecodeLedger,
    ) -> std::result::Result<NativeOwned<T>, Error>,
) -> std::result::Result<NativeOwned<Supplied<T>>, Error> {
    let p = f::child(plan, held, key, s::PRESENCE, ledger)?;
    let state = f::text(&p, held, Presence, ledger)?;
    if matches!(state.as_str(), "absent" | "null")
        && p.payload().has(Value)
        && !f::is_null(&p, held, Value)?
    {
        return Err(Error::Type);
    }
    match state.as_str() {
        "absent" => Ok(NativeOwned::<T>::not_supplied(ledger)?),
        "null" => Ok(NativeOwned::<T>::null(ledger)?),
        "value" => {
            if semantic {
                f::semantic_value(&p, held, ledger)?;
            }
            Ok(decode(&p, held, ledger)?.presence())
        }
        _ => Err(Error::Variant),
    }
}
fn scalar_presence(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<Supplied<bool>, Error> {
    let p = f::child(plan, held, Redacted, s::PRESENCE, ledger)?;
    let state = f::text(&p, held, Presence, ledger)?;
    if matches!(state.as_str(), "absent" | "null")
        && p.payload().has(Value)
        && !f::is_null(&p, held, Value)?
    {
        return Err(Error::Type);
    }
    Ok(match state.as_str() {
        "absent" => Supplied::Absent,
        "null" => Supplied::Null,
        "value" => Supplied::Value(f::boolean(&p, held, Value, ledger)?),
        _ => return Err(Error::Variant),
    })
}
fn outcome(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<DisplayOutcome>, Error> {
    // The unit variant is a JSON string; nonunit variants have one external tag.
    let string = f::starts(plan, held, Value, b'"')?;
    if string {
        let value = f::text(plan, held, Value, ledger)?;
        return if value.as_str() == "Pending" {
            Ok(nocterm_ai::model::native::tools::pending(ledger)?)
        } else {
            Err(Error::Variant)
        };
    }
    let p = f::child(plan, held, Value, s::OUTCOME, ledger)?;
    match (
        p.payload().has(OutcomePending),
        p.payload().has(OutcomeOk),
        p.payload().has(OutcomeErr),
    ) {
        (true, false, false) if f::is_null(&p, held, OutcomePending)? => {
            Ok(nocterm_ai::model::native::tools::pending(ledger)?)
        }
        (false, true, false) => Ok(nocterm_ai::model::native::tools::succeeded(
            nocterm_ai::model::native::content::json(
                f::node(&p, held, OutcomeOk, ledger)?,
                ledger,
            )?,
        )),
        (false, false, true) => Ok(nocterm_ai::model::native::tools::failed(f::text(
            &p, held, OutcomeErr, ledger,
        )?)),
        _ => Err(Error::Variant),
    }
}
