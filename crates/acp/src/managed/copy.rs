//! Distinct neutral copies use the same domain-owned paid construction policy.
use super::{budget::SourceBudget, error::DecodeError};
use nocterm_ai::model::{
    json::deferred::JsonDecodeError,
    native::{NativeCopies, NativeOwned},
};
type Error = DecodeError<JsonDecodeError>;
pub(super) fn text(budget: &SourceBudget, value: &str) -> Result<NativeOwned<String>, Error> {
    Ok(NativeCopies::new(budget.ledger()).text(value)?)
}
