//! Legacy contextual defaults apply only to semantic field errors.
mod content;
pub(super) mod document;
mod tools;
use super::{
    Error, fields as f, legacy_value,
    schema::{Schema, Scope},
};
use crate::loading::{
    held_file::{HeldFile, plan::AuthenticatedPlan},
    json_fields::Key,
};
use nocterm_ai::model::{
    extensions::Extensions,
    json::deferred::{DecodeLedger, native::NativeKind},
    native::NativeOwned,
};
pub(super) fn fallback<T>(
    result: std::result::Result<T, Error>,
    default: impl FnOnce() -> std::result::Result<T, Error>,
) -> std::result::Result<T, Error> {
    match result {
        Err(error) if error.defaultable() => default(),
        other => other,
    }
}
fn optional_text(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<Option<String>>, Error> {
    fallback(f::optional_text(plan, held, key, ledger), || {
        Ok(NativeOwned::<String>::absent(ledger)?)
    })
}
fn opaque(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
    map: bool,
) -> std::result::Result<Option<nocterm_ai::model::json::deferred::native::NativeNode>, Error> {
    let Some(field) = plan.payload().field(key) else {
        return Ok(None);
    };
    if f::is_null(plan, held, key)? || (map && !f::starts(plan, held, key, b'{')?) {
        return Ok(None);
    }
    let parsed = legacy_value::authenticated(plan, held, |_| field.span(), ledger, map)?;
    match parsed {
        Ok(node) => Ok(Some(node)),
        Err(error) if error.defaultable() => Ok(None),
        Err(error) => Err(error.into()),
    }
}
fn extensions(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<Option<Extensions>>, Error> {
    let Some(value) = opaque(plan, held, key, ledger, true)? else {
        return Ok(NativeOwned::<Extensions>::absent(ledger)?);
    };
    if value.kind() != NativeKind::Object {
        return Ok(NativeOwned::<Extensions>::absent(ledger)?);
    }
    Ok(NativeOwned::supplied(
        nocterm_ai::model::native::content::extensions(value, ledger)?,
    ))
}
fn optional<T>(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
    schema: Schema,
    decode: impl FnOnce(
        &AuthenticatedPlan<Scope>,
        &mut HeldFile,
        &DecodeLedger,
    ) -> std::result::Result<NativeOwned<T>, Error>,
) -> std::result::Result<NativeOwned<Option<T>>, Error> {
    if !plan.payload().has(key) || f::is_null(plan, held, key)? {
        return Ok(NativeOwned::<T>::absent(ledger)?);
    }
    let decoded = f::child(plan, held, key, schema, ledger).and_then(|p| decode(&p, held, ledger));
    fallback(decoded.map(NativeOwned::supplied), || {
        Ok(NativeOwned::<T>::absent(ledger)?)
    })
}

fn raw_node(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
) -> std::result::Result<nocterm_ai::model::json::deferred::native::NativeNode, Error> {
    let field = plan.payload().required(key)?;
    Ok(legacy_value::authenticated(
        plan,
        held,
        |_| field.span(),
        ledger,
        false,
    )??)
}
fn resource_text(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<String>, Error> {
    let node = raw_node(plan, held, key, ledger)?;
    if node.kind() != NativeKind::String {
        return Err(Error::Type);
    }
    Ok(NativeOwned::text(node.into_text()?))
}
