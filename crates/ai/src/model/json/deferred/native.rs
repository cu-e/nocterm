//! Consuming access to a fresh controlled literal tree, before shared publication.
//! No arbitrary Value, callback or caller-provided allocation cost enters this API.
mod capture;
pub use super::literal::native_object::NativeObject;
use super::{DecodeLedger, DecodedJson, JsonDecodeError};
use crate::model::json::lifetime::Shared;
use crate::model::json::{OwnedJson, lifecycle::GuardedValue};
use capture::Layout;

/// A fresh payload and the private, metered layout of its actual construction.
/// Cloning and raw Value extraction are deliberately absent.
pub struct NativeNode {
    value: GuardedValue,
    layout: Shared<Layout>,
    index: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeKind {
    Null,
    Boolean,
    Number,
    String,
    Array,
    Object,
}
impl NativeNode {
    pub(super) fn capture(
        value: GuardedValue,
        ledger: &DecodeLedger,
    ) -> Result<Self, JsonDecodeError> {
        if value.native_cleanup().is_none() {
            return Err(JsonDecodeError::CleanupFrontier);
        }
        let layout = Layout::capture(&value, ledger)?;
        Ok(Self {
            value,
            layout,
            index: 0,
        })
    }
    pub(in crate::model::json::deferred) fn into_guarded(
        self,
        ledger: &DecodeLedger,
    ) -> Result<GuardedValue, JsonDecodeError> {
        if !self.value.belongs_to(ledger) {
            return Err(JsonDecodeError::ForeignLedger);
        }
        Ok(self.value)
    }
    pub fn kind(&self) -> NativeKind {
        match self.value.native_value() {
            serde_json::Value::Null => NativeKind::Null,
            serde_json::Value::Bool(_) => NativeKind::Boolean,
            serde_json::Value::Number(_) => NativeKind::Number,
            serde_json::Value::String(_) => NativeKind::String,
            serde_json::Value::Array(_) => NativeKind::Array,
            serde_json::Value::Object(_) => NativeKind::Object,
        }
    }
    pub fn container_len(&self) -> Option<usize> {
        match self.value.native_value() {
            serde_json::Value::Array(values) => Some(values.len()),
            serde_json::Value::Object(values) => Some(values.len()),
            _ => None,
        }
    }
    pub fn boolean(&self) -> Option<bool> {
        self.value.native_value().as_bool()
    }
    pub fn number(&self) -> Option<&serde_json::Number> {
        self.value.native_value().as_number()
    }
    pub fn text(&self) -> Option<&str> {
        self.value.native_value().as_str()
    }
    /// Opaque JSON is adopted without recursive normalization or a depth policy.
    /// The ordinary shared header/census admission remains authoritative.
    pub fn into_decoded(self, ledger: &DecodeLedger) -> Result<DecodedJson, JsonDecodeError> {
        let value = OwnedJson::from_admitted(self.value, ledger)?;
        Ok(DecodedJson { value })
    }
}

use crate::model::json::lifecycle::NativeCursor;
pub use crate::model::json::lifecycle::NativeText;
/// A consuming container iterator retains its parent backing throughout iteration.
pub struct NativeChildren {
    cursor: NativeCursor,
    layout: Shared<Layout>,
    next: Option<usize>,
    object: bool,
}
impl NativeNode {
    pub fn into_text(self) -> Result<NativeText, JsonDecodeError> {
        self.value.into_native_text()
    }
    pub fn into_children(self) -> Result<NativeChildren, JsonDecodeError> {
        let object = match self.kind() {
            NativeKind::Object => true,
            NativeKind::Array => false,
            _ => return Err(JsonDecodeError::ConstructionOrder),
        };
        let next = self.layout.fact(self.index).first;
        let cursor = self.value.into_native_cursor()?;
        Ok(NativeChildren {
            cursor,
            layout: self.layout,
            next,
            object,
        })
    }
}
impl NativeChildren {
    /// Object entries include their original guarded key; array entries use None.
    /// Exhaustion is explicit because splitting a genuine token is fallible.
    pub fn next_child(
        &mut self,
    ) -> Result<Option<(Option<NativeText>, NativeNode)>, JsonDecodeError> {
        let Some(index) = self.next else {
            return Ok(None);
        };
        let fact = self.layout.fact(index);
        let bytes = fact.bytes;
        let frontier = fact.frontier;
        let key_bytes = fact.key_bytes;
        let next = fact.next;
        let (key, value) = self
            .cursor
            .next(bytes, frontier, key_bytes)?
            .ok_or(JsonDecodeError::ConstructionOrder)?;
        if key.is_some() != self.object {
            return Err(JsonDecodeError::ConstructionOrder);
        }
        self.next = next;
        Ok(Some((
            key,
            NativeNode {
                value,
                layout: self.layout.clone(),
                index,
            },
        )))
    }
}

#[cfg(test)]
mod tests;
