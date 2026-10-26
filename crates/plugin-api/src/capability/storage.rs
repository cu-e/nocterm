//! `storage.set` and `storage.remove`.

use alloc::string::String;
use serde::{Deserialize, Serialize};

use super::{Capability, sealed};
use crate::{LimitError, error::key_syntax};

/// Longest storage key.
pub const MAX_KEY: usize = 64;
/// Longest stored value, in bytes.
pub const MAX_VALUE: usize = 4 * 1024;
/// Most bytes one plugin stores in total.
pub const MAX_TOTAL: usize = 64 * 1024;

/// Stores a value under a key of the plugin's own data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Set {
    pub key: String,
    pub value: String,
}

impl Set {
    pub fn check(&self) -> Result<(), LimitError> {
        key_syntax(&self.key, MAX_KEY, "key")?;
        if self.value.len() > MAX_VALUE {
            return Err(LimitError::TooLong {
                field: "value",
                max: MAX_VALUE,
            });
        }
        Ok(())
    }
}

/// Marker of `storage.set`.
pub struct StoreSet;

impl sealed::Sealed for StoreSet {}

impl Capability for StoreSet {
    const ID: &'static str = "storage.set";
    type Value = Set;
}

/// Removes a key of the plugin's own data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Remove {
    pub key: String,
}

impl Remove {
    pub fn check(&self) -> Result<(), LimitError> {
        key_syntax(&self.key, MAX_KEY, "key")
    }
}

/// Marker of `storage.remove`.
pub struct StoreRemove;

impl sealed::Sealed for StoreRemove {}

impl Capability for StoreRemove {
    const ID: &'static str = "storage.remove";
    type Value = Remove;
}
