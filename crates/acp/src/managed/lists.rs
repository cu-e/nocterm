//! Schema skip/default policy is separated from physical typed-vector admission.
use super::{error::DecodeError, value::Value};
use nocterm_ai::model::{
    json::deferred::JsonDecodeError,
    native::{NativeList, NativeOwned},
};
type Error = DecodeError<JsonDecodeError>;

pub(super) fn skipped<T>(
    value: Value<'_>,
    valid: impl Fn(&Value<'_>) -> Result<bool, Error>,
    build: impl Fn(Value<'_>) -> Result<NativeOwned<T>, Error>,
) -> Result<NativeOwned<Vec<T>>, Error> {
    if !value.raw().starts_with('[') {
        return Ok(NativeList::new(value.ledger(), 0)?.finish());
    }
    let mut count = 0;
    let mut elements = value.elements()?;
    while let Some(item) = elements.next()? {
        if valid(&item)? {
            count += 1;
        }
    }
    let mut result = NativeList::new(value.ledger(), count)?;
    let mut elements = value.elements()?;
    while let Some(item) = elements.next()? {
        if valid(&item)? {
            result.push(build(item)?)?;
        }
    }
    Ok(result.finish())
}
/// A schema-invalid list member is skipped; actual reservation refusal always
/// remains an error and cannot be mistaken for malformed optional input.
pub(super) fn shape<T>(result: Result<T, Error>) -> Result<Option<T>, Error> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(DecodeError::Field(_) | DecodeError::Syntax(_)) => Ok(None),
        Err(error) => Err(error),
    }
}
