//! Sealed native construction keeps exclusive payload credit with concrete DTOs.
//! Adapter grammar stays outside this module. No generic raw-value constructor,
//! unwrap or arbitrary conversion callback is exposed to the adapter.
//! Inherent methods are generic capability operations only; payload
//! construction and transitions are free functions in the area modules.
mod bytes;
pub use bytes::NativeBytes;
pub mod choices;
pub mod content;
mod copies;
pub use copies::NativeCopies;
pub mod controls;
pub mod usage;
pub use choices::NativeChoices;
pub mod auth;
pub mod claims;
pub mod notice;
pub mod patch;
mod path;
pub mod remote;
pub mod workspace;
pub use path::NativePathError;
pub mod session;
pub mod tools;
use super::json::deferred::{
    DecodeLedger, JsonDecodeError, ledger::Reservation, native::NativeText,
};
use std::mem::size_of;
pub use tools::NativeToolHeader;

/// A concrete neutral value and its genuine construction-phase allocation.
/// This capability is consumed by named DTO constructors or history admission.
#[must_use = "retain the capability until the concrete value is adopted"]
pub struct NativeOwned<T> {
    value: T,
    allocation: Reservation,
}
impl<T> NativeOwned<T> {
    pub(crate) fn into_parts(self) -> (T, Reservation) {
        (self.value, self.allocation)
    }
    pub(crate) fn from_parts(value: T, allocation: Reservation) -> Self {
        Self { value, allocation }
    }
    /// Borrowed inspection; ownership and its allocation stay together.
    pub fn get(&self) -> &T {
        &self.value
    }
    pub(crate) fn join<U>(
        self,
        other: NativeOwned<U>,
    ) -> Result<NativeOwned<(T, U)>, JsonDecodeError> {
        if !other.allocation.belongs_to(self.ledger()) {
            return Err(JsonDecodeError::ForeignLedger);
        }
        // One ledger's live count already bounds the sum of its disjoint tokens.
        self.allocation
            .bytes()
            .checked_add(other.allocation.bytes())
            .ok_or(JsonDecodeError::Arithmetic)?;
        let (left, mut allocation) = self.into_parts();
        let (right, right_allocation) = other.into_parts();
        allocation
            .merge(right_allocation)
            .expect("validated same-ledger sum");
        Ok(NativeOwned {
            value: (left, right),
            allocation,
        })
    }
    pub(crate) fn belongs_to_document(
        &self,
        scope: &super::retention::document::DocumentRetention,
    ) -> bool {
        self.allocation.belongs_to_document(scope)
    }
    pub fn belongs_to_ledger(&self, ledger: &DecodeLedger) -> bool {
        self.allocation.belongs_to(ledger)
    }
    pub fn ledger(&self) -> &DecodeLedger {
        self.allocation.original_ledger()
    }
    pub fn presence(self) -> NativeOwned<super::controls::Presence<T>> {
        NativeOwned {
            value: super::controls::Presence::Value(self.value),
            allocation: self.allocation,
        }
    }
    pub fn null(
        ledger: &DecodeLedger,
    ) -> Result<NativeOwned<super::controls::Presence<T>>, JsonDecodeError> {
        Ok(NativeOwned {
            value: super::controls::Presence::Null,
            allocation: ledger.reserve(0)?,
        })
    }
    pub fn not_supplied(
        ledger: &DecodeLedger,
    ) -> Result<NativeOwned<super::controls::Presence<T>>, JsonDecodeError> {
        Ok(NativeOwned {
            value: super::controls::Presence::Absent,
            allocation: ledger.reserve(0)?,
        })
    }
    pub fn supplied(value: Self) -> NativeOwned<Option<T>> {
        NativeOwned {
            value: Some(value.value),
            allocation: value.allocation,
        }
    }
    pub fn absent(ledger: &DecodeLedger) -> Result<NativeOwned<Option<T>>, JsonDecodeError> {
        Ok(NativeOwned {
            value: None,
            allocation: ledger.reserve(0)?,
        })
    }
}
impl<T: Copy> NativeOwned<T> {
    /// Inline scalar facts allocate no owned heap. Concrete DTO constructors
    /// accept only their declared scalar types, never raw pointers as authority.
    pub fn scalar(value: T, ledger: &DecodeLedger) -> Result<Self, JsonDecodeError> {
        Ok(Self::from_parts(value, ledger.reserve(0)?))
    }
}
impl NativeOwned<String> {
    pub fn text(text: NativeText) -> Self {
        let (value, allocation) = text.into_parts();
        Self { value, allocation }
    }
    pub fn as_str(&self) -> &str {
        &self.value
    }
    pub fn as_text(&self) -> &str {
        &self.value
    }
}
impl NativeOwned<Option<String>> {
    pub fn as_optional_str(&self) -> Option<&str> {
        self.value.as_deref()
    }
}
impl NativeOwned<Option<super::json::OwnedJson>> {
    pub fn as_optional_json(&self) -> Option<&super::json::OwnedJson> {
        self.value.as_ref()
    }
}
impl NativeOwned<Option<std::path::PathBuf>> {
    pub fn as_optional_path(&self) -> Option<&std::path::Path> {
        self.get().as_deref()
    }
}
impl NativeOwned<std::path::PathBuf> {
    pub fn copy_path(
        ledger: &DecodeLedger,
        value: &std::path::Path,
    ) -> Result<Self, JsonDecodeError> {
        path::PathConstruction::new(ledger).copy(value)
    }
    pub fn join_path(
        ledger: &DecodeLedger,
        base: &std::path::Path,
        right: &std::path::Path,
    ) -> Result<Self, NativePathError> {
        path::PathConstruction::new(ledger).join(base, right)
    }
    pub fn as_path(&self) -> &std::path::Path {
        self.value.as_path()
    }
}
impl NativeOwned<crate::history::SavedDocument> {
    pub(crate) fn replace_archived_draft(
        &mut self,
        draft: Option<String>,
        updated: u64,
        allocation: &mut Reservation,
    ) -> Result<(), JsonDecodeError> {
        let old_bytes = self
            .value
            .composer
            .draft
            .as_ref()
            .map_or(0, String::capacity);
        let old_allocation = self.allocation.split(old_bytes)?;
        self.allocation.merge_from(allocation)?;
        drop(std::mem::replace(&mut self.value.composer.draft, draft));
        drop(old_allocation);
        self.value.metadata.updated = updated;
        Ok(())
    }
}
/// A typed vector prepays real final capacity before allocation and never grows
/// implicitly through a serde visitor's unknown element layout.
pub struct NativeList<T> {
    values: Vec<T>,
    allocation: Reservation,
}
impl<T> NativeList<T> {
    pub fn new(ledger: &DecodeLedger, capacity: usize) -> Result<Self, JsonDecodeError> {
        let bytes = capacity
            .checked_mul(size_of::<T>())
            .ok_or(JsonDecodeError::Arithmetic)?;
        let allocation = ledger.reserve(bytes)?;
        let mut values = Vec::new();
        values.try_reserve_exact(capacity)?;
        if size_of::<T>() != 0 && values.capacity() != capacity {
            return Err(JsonDecodeError::CapacityProfile);
        }
        Ok(Self { values, allocation })
    }
    pub fn push(&mut self, mut value: NativeOwned<T>) -> Result<(), JsonDecodeError> {
        if self.values.len() == self.values.capacity() {
            return Err(JsonDecodeError::ConstructionOrder);
        }
        self.allocation.merge_from(&mut value.allocation)?;
        self.values.push(value.value);
        Ok(())
    }
    pub fn finish(self) -> NativeOwned<Vec<T>> {
        NativeOwned {
            value: self.values,
            allocation: self.allocation,
        }
    }
    pub(crate) fn contains(&self, value: &T) -> bool
    where
        T: PartialEq,
    {
        self.values.contains(value)
    }
}
