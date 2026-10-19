//! Immutable JSON with actual shared-allocation identity and private reservations.
//! Checked planned literal construction has guarded iterative cleanup. Untracked
//! normalize/Deserialize and legacy DeferredNode traversal remain recursive seams;
//! generic Serialize/equality/native SDK traversal are not made iterative here.
mod containers;
pub mod deferred;
mod lifecycle;
pub(crate) mod lifetime;
mod retained;
pub use retained::RetainedAllocation;
#[cfg(test)]
mod tests;
use super::diagnostic::ModelError;
use super::retention::RetentionAuthority;
use deferred::{DecodeLedger, JsonDecodeError};
use lifecycle::{Backing, GuardedValue};
use lifetime::HeaderLease;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fmt,
    sync::{Arc, Weak},
};

/// Weak drops before lease: inline credit outlives the actual shared allocation.
#[derive(Clone, Debug)]
pub struct JsonReceipt {
    retained: usize,
    identity: Weak<Backing>,
    lease: HeaderLease,
}
impl JsonReceipt {
    pub fn retained_heap_bytes(&self) -> usize {
        self.retained
    }
    /// Root and authentic ledger are separate borrowed allocation components.
    /// Existing root-only cost queries deliberately exclude ledger metadata.
    pub fn retained_allocations(&self) -> [Option<RetainedAllocation<'_>>; 2] {
        [
            Some(RetainedAllocation::root(self)),
            self.lease
                .original_ledger()
                .map(DecodeLedger::allocation_component),
        ]
    }
    pub fn matches(&self, value: &OwnedJson) -> bool {
        self.identity.ptr_eq(&Arc::downgrade(&value.root))
    }
    /// Last strong can be running Drop. Only actual retirement publication permits
    /// replacing full retained cost by the genuine remaining identity allocations.
    pub fn retained_identity_bytes(&self) -> Option<usize> {
        if self.lease.payload_retired() {
            self.lease.retired_bytes()
        } else {
            Some(self.retained)
        }
    }
}
/// Field order retires root before receipt/Weak/inline lease, including aliases.
#[derive(Clone, Debug)]
pub struct OwnedJson {
    root: Arc<Backing>,
    receipt: JsonReceipt,
}
impl OwnedJson {
    /// Canonical intake does not account for old caller reserve or make its
    /// recursive normalization/error disposal a bounded managed decoder.
    pub fn from_untracked(
        authority: &RetentionAuthority,
        value: Value,
    ) -> Result<Self, ModelError> {
        let (value, retained) = containers::normalize(value)?;
        let retained = retained
            .checked_add(Self::identity_allocation_bytes().ok_or(ModelError::JsonArithmetic)?)
            .ok_or(ModelError::JsonArithmetic)?;
        let lease = HeaderLease::untracked(
            authority,
            retained - Self::identity_allocation_bytes().ok_or(ModelError::JsonArithmetic)?,
        );
        let root = Arc::new(Backing::untracked(value, lease.clone()));
        lease.bind(
            &root,
            retained - Self::identity_allocation_bytes().ok_or(ModelError::JsonArithmetic)?,
            None,
            None,
        );
        let receipt = JsonReceipt {
            retained,
            identity: Arc::downgrade(&root),
            lease,
        };
        Ok(Self { root, receipt })
    }
    /// One already guarded owner carries its authentic token through every error.
    /// No arbitrary caller Value or claimed receipt amount can enter this method.
    fn from_admitted(
        mut value: GuardedValue,
        ledger: &DecodeLedger,
    ) -> Result<Self, JsonDecodeError> {
        if !value.belongs_to(ledger) {
            return Err(JsonDecodeError::ForeignLedger);
        }
        let payload_bytes = value.exclusive_bytes();
        let lease = HeaderLease::admitted(ledger, &value)?;
        let backing_allocation = ledger.reserve(lifetime::backing_bytes())?;
        let payload_allocation = value.take_payload_allocation()?;
        let root = Arc::new(Backing::admitted(value, lease.clone()));
        lease.bind(
            &root,
            payload_bytes,
            Some(backing_allocation),
            Some(payload_allocation),
        );
        let retained = lease
            .retained_root_bytes()
            .ok_or(JsonDecodeError::Arithmetic)?;
        let receipt = JsonReceipt {
            retained,
            identity: Arc::downgrade(&root),
            lease,
        };
        Ok(Self { root, receipt })
    }
    pub(crate) fn identity_allocation_bytes() -> Option<usize> {
        lifetime::identity_bytes()
    }
    fn belongs_to(&self, ledger: &DecodeLedger) -> bool {
        self.root.belongs_to(ledger)
    }
    pub fn authority(&self) -> &RetentionAuthority {
        self.receipt.lease.authority()
    }
    pub fn as_value(&self) -> &Value {
        self.root.value()
    }
    pub fn receipt(&self) -> &JsonReceipt {
        &self.receipt
    }
    pub fn retained_heap_bytes(&self) -> usize {
        self.receipt.retained
    }
    pub fn retained_allocations(&self) -> [Option<RetainedAllocation<'_>>; 2] {
        self.receipt.retained_allocations()
    }

    /// Borrowed queries do not authorize a caller's new raw Value clone as managed.
    pub fn shares_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.root, &other.root)
    }
}
impl std::ops::Deref for OwnedJson {
    type Target = Value;
    fn deref(&self) -> &Value {
        self.as_value()
    }
}
impl PartialEq for OwnedJson {
    fn eq(&self, other: &Self) -> bool {
        self.as_value() == other.as_value()
    }
}
impl PartialEq<Value> for OwnedJson {
    fn eq(&self, other: &Value) -> bool {
        self.as_value() == other
    }
}
impl fmt::Display for OwnedJson {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_value().fmt(f)
    }
}
impl Serialize for OwnedJson {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_value().serialize(serializer)
    }
}
/// Compatibility intake with an explicitly supplied context. Native bounded
/// decoding uses the admitted builder; this seed does not claim parser peak safety.
pub struct OwnedJsonSeed<'a>(pub &'a RetentionAuthority);
impl<'de> serde::de::DeserializeSeed<'de> for OwnedJsonSeed<'_> {
    type Value = OwnedJson;
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<Self::Value, D::Error> {
        OwnedJson::from_untracked(self.0, Value::deserialize(decoder)?)
            .map_err(serde::de::Error::custom)
    }
}

impl crate::model::retention::graph::sealed::Source for JsonReceipt {}
impl crate::model::retention::graph::OwnerGraph for JsonReceipt {
    fn count(
        &self,
        count: &mut crate::model::retention::graph::Count<'_, '_>,
    ) -> Result<(), crate::model::retention::RetentionError> {
        self.lease.count(count)
    }
    fn cleanup(&self, permit: &crate::model::retention::CensusPermit<'_>) {
        self.lease.cleanup(permit);
    }

    fn adopt(&self, publication: &mut crate::model::retention::document::Publication<'_>) {
        self.lease.adopt(publication);
    }
}
