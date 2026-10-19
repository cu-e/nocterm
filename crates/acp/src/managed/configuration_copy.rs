//! Cache and delivered controls use canonical distinct paid neutral copies.
use super::{budget::SourceBudget, error::DecodeError};
use nocterm_ai::model::{
    controls::ConfigOption,
    json::deferred::JsonDecodeError,
    native::{NativeCopies, NativeOwned},
};
type Error = DecodeError<JsonDecodeError>;
pub(super) fn options(
    budget: &SourceBudget,
    values: &[ConfigOption],
    override_current: Option<(&str, &str)>,
) -> Result<NativeOwned<Vec<ConfigOption>>, Error> {
    Ok(NativeCopies::new(budget.ledger()).configurations(values, override_current)?)
}
pub(super) fn option(
    budget: &SourceBudget,
    value: &ConfigOption,
    override_current: Option<(&str, &str)>,
) -> Result<NativeOwned<ConfigOption>, Error> {
    Ok(NativeCopies::new(budget.ledger()).option(value, override_current)?)
}
