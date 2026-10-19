//! Field readers retain guarded payloads through the concrete owner transition.
use super::{
    Error,
    schema::{self, Scope},
};
use crate::loading::{
    held_file::{HeldFile, plan::AuthenticatedPlan},
    json_fields::Key,
    literal_reader::decode_authenticated_native,
};
use nocterm_ai::model::{
    extensions::Extensions,
    json::deferred::{
        DecodeLedger,
        native::{NativeKind, NativeNode},
    },
    native::NativeOwned,
};
use std::path::PathBuf;
pub(super) fn node(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeNode, Error> {
    if let Some(node) = plan.payload().take_native(key) {
        return node;
    }
    let field = plan.payload().required(key)?;
    Ok(decode_authenticated_native(
        plan,
        held,
        |_| field.span(),
        ledger,
    )??)
}
pub(super) fn text(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<String>, Error> {
    if !starts(plan, held, key, b'"')? {
        return Err(Error::Type);
    }
    let value = node(plan, held, key, ledger)?;
    if value.kind() != NativeKind::String {
        return Err(Error::Type);
    }
    Ok(NativeOwned::text(value.into_text()?))
}
pub(super) fn optional<T>(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
    read: impl FnOnce(
        &AuthenticatedPlan<Scope>,
        &mut HeldFile,
        Key,
        &DecodeLedger,
    ) -> std::result::Result<NativeOwned<T>, Error>,
) -> std::result::Result<NativeOwned<Option<T>>, Error> {
    if !plan.payload().has(key) {
        return Ok(NativeOwned::<T>::absent(ledger)?);
    }
    if is_null(plan, held, key)? {
        return Ok(NativeOwned::<T>::absent(ledger)?);
    }
    Ok(NativeOwned::supplied(read(plan, held, key, ledger)?))
}
pub(super) fn optional_text(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<Option<String>>, Error> {
    if !plan.payload().has(key) {
        return Ok(NativeOwned::<String>::absent(ledger)?);
    }
    if is_null(plan, held, key)? {
        return Ok(NativeOwned::<String>::absent(ledger)?);
    }
    if !starts(plan, held, key, b'"')? {
        return Err(Error::Type);
    }
    let value = node(plan, held, key, ledger)?;
    if value.kind() == NativeKind::Null {
        Ok(NativeOwned::<String>::absent(ledger)?)
    } else {
        if value.kind() != NativeKind::String {
            return Err(Error::Type);
        }
        Ok(NativeOwned::supplied(NativeOwned::text(value.into_text()?)))
    }
}
pub(super) fn extensions(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<Option<Extensions>>, Error> {
    if !plan.payload().has(key) {
        return Ok(NativeOwned::<Extensions>::absent(ledger)?);
    }
    let value = node(plan, held, key, ledger)?;
    if value.kind() == NativeKind::Null {
        Ok(NativeOwned::<Extensions>::absent(ledger)?)
    } else {
        if value.kind() != NativeKind::Object {
            return Err(Error::Type);
        }
        Ok(NativeOwned::supplied(
            nocterm_ai::model::native::content::extensions(value, ledger)?,
        ))
    }
}
pub(super) fn path(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<PathBuf>, Error> {
    Ok(nocterm_ai::model::native::tools::path(text(
        plan, held, key, ledger,
    )?)?)
}
pub(super) fn boolean(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
) -> std::result::Result<bool, Error> {
    node(plan, held, key, ledger)?.boolean().ok_or(Error::Type)
}
pub(super) fn unsigned(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
) -> std::result::Result<u64, Error> {
    node(plan, held, key, ledger)?
        .number()
        .and_then(|v| v.as_u64())
        .ok_or(Error::Type)
}
pub(super) fn optional_number<T>(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
    read: impl FnOnce(&serde_json::Number) -> Option<T>,
) -> std::result::Result<Option<T>, Error> {
    if !plan.payload().has(key) {
        return Ok(None);
    }
    let value = node(plan, held, key, ledger)?;
    if value.kind() == NativeKind::Null {
        return Ok(None);
    }
    value.number().and_then(read).map(Some).ok_or(Error::Type)
}
pub(super) fn tag(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<String>, Error> {
    let tag = text(plan, held, Key::Kind, ledger)?;
    semantic_value(plan, held, ledger)?;
    Ok(tag)
}
pub(super) fn child(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    schema: schema::Schema,
    ledger: &DecodeLedger,
) -> std::result::Result<AuthenticatedPlan<Scope>, Error> {
    schema::nested(plan, held, key, schema, ledger)
}

pub(super) fn is_null(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
) -> std::result::Result<bool, Error> {
    if let Some(kind) = plan.payload().native_kind(key) {
        return Ok(kind? == NativeKind::Null);
    }
    let field = plan.payload().required(key)?;
    Ok(plan.replay(
        held,
        |_| field.span(),
        |reader| {
            let mut first = [0];
            loop {
                match reader.read(&mut first) {
                    Ok(0) => return false,
                    Ok(_) => {
                        if !first[0].is_ascii_whitespace() {
                            return first[0] == b'n';
                        }
                    }
                    Err(_) => return false,
                }
            }
        },
    )?)
}
pub(super) fn starts(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    expected: u8,
) -> std::result::Result<bool, Error> {
    if let Some(kind) = plan.payload().native_kind(key) {
        return Ok(kind?
            == match expected {
                b'"' => NativeKind::String,
                b'{' => NativeKind::Object,
                b'[' => NativeKind::Array,
                b'n' => NativeKind::Null,
                _ => return Ok(false),
            });
    }
    let field = plan.payload().required(key)?;
    Ok(plan.replay(
        held,
        |_| field.span(),
        |reader| {
            let mut first = [0];
            loop {
                match reader.read(&mut first) {
                    Ok(0) | Err(_) => return false,
                    Ok(_) => {
                        if !first[0].is_ascii_whitespace() {
                            return first[0] == expected;
                        }
                    }
                }
            }
        },
    )?)
}

pub(super) fn semantic_value(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<(), Error> {
    if plan.payload().is_native() || !plan.payload().has(Key::Value) {
        return Ok(());
    }
    let field = plan.payload().required(Key::Value)?;
    let value = super::legacy_value::version2(plan, held, |_| field.span(), ledger)??;
    plan.payload().capture(value)
}
