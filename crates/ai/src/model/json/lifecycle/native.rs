//! Moving native iteration keeps the original table/keys/token until retirement.
use super::{Cleanup, GuardedValue};
use crate::model::json::deferred::{JsonDecodeError, ledger::Reservation};
use serde_json::Value;
use std::mem;

/// Text remains paired with its authentic allocation until a typed owner adopts it.
/// Borrowing does not authorize a caller's new String clone as controlled memory.
pub struct NativeText {
    text: String,
    allocation: Reservation,
}
impl NativeText {
    pub(crate) fn into_parts(self) -> (String, Reservation) {
        (self.text, self.allocation)
    }
    pub(crate) fn belongs_to(&self, ledger: &crate::model::json::deferred::DecodeLedger) -> bool {
        self.allocation.belongs_to(ledger)
    }
    pub fn as_str(&self) -> &str {
        &self.text
    }
    pub fn capacity(&self) -> usize {
        self.text.capacity()
    }
}
enum Values {
    Array(std::vec::IntoIter<Value>),
    Object(serde_json::map::IntoIter),
}
pub(in crate::model::json) struct NativeCursor {
    values: Option<Values>,
    cleanup: Cleanup,
    allocation: Reservation,
}
impl GuardedValue {
    pub(in crate::model::json) fn into_native_cursor(
        mut self,
    ) -> Result<NativeCursor, JsonDecodeError> {
        if !matches!(self.value, Value::Array(_) | Value::Object(_)) {
            return Err(JsonDecodeError::ConstructionOrder);
        }
        let cleanup = self
            .cleanup
            .clone()
            .ok_or(JsonDecodeError::CleanupFrontier)?;
        let allocation = self.allocation.split(self.allocation.bytes())?;
        let values = match mem::take(&mut self.value) {
            Value::Array(values) => Values::Array(values.into_iter()),
            Value::Object(values) => Values::Object(values.into_iter()),
            _ => unreachable!("container kind checked before taking payload"),
        };
        Ok(NativeCursor {
            values: Some(values),
            cleanup,
            allocation,
        })
    }
    pub(in crate::model::json) fn into_native_text(
        mut self,
    ) -> Result<NativeText, JsonDecodeError> {
        if !self.value.is_string() {
            return Err(JsonDecodeError::ConstructionOrder);
        }
        let allocation = self.allocation.split(self.allocation.bytes())?;
        let Value::String(text) = mem::take(&mut self.value) else {
            unreachable!("string kind checked")
        };
        Ok(NativeText { text, allocation })
    }
}
impl NativeCursor {
    /// Costs originate only in the private capture of this exact controlled tree.
    pub(in crate::model::json) fn next(
        &mut self,
        bytes: usize,
        frontier: usize,
        key_bytes: usize,
    ) -> Result<Option<(Option<NativeText>, GuardedValue)>, JsonDecodeError> {
        let total = bytes
            .checked_add(key_bytes)
            .ok_or(JsonDecodeError::Arithmetic)?;
        let mut key = self.allocation.split(total)?;
        let mut child = key
            .split(bytes)
            .expect("checked child is within its combined private token");
        let next = match self.values.as_mut().expect("live native iterator") {
            Values::Array(values) => values.next().map(|value| (None, value)),
            Values::Object(values) => values.next().map(|(key, value)| (Some(key), value)),
        };
        let Some((text, value)) = next else {
            // No raw payload escaped if the sealed capture/iterator disagree.
            self.allocation.merge_from(&mut child)?;
            self.allocation.merge(key)?;
            return Ok(None);
        };
        let text = match text {
            Some(text) => Some(NativeText {
                text,
                allocation: key,
            }),
            None => {
                drop(key);
                None
            }
        };
        Ok(Some((
            text,
            GuardedValue::new(value, frontier, Some(self.cleanup.clone()), child),
        )))
    }
}
impl Drop for NativeCursor {
    fn drop(&mut self) {
        if let Some(values) = self.values.take() {
            match values {
                Values::Array(values) => {
                    for value in values {
                        self.cleanup.drain(value);
                    }
                }
                Values::Object(values) => {
                    for (key, value) in values {
                        drop(key);
                        self.cleanup.drain(value);
                    }
                }
            }
        }
        // IntoIter frees its original backing before the payload token retires.
    }
}
