use super::json::{
    deferred::{JsonDecodeError, ledger::Reservation},
    lifetime::Shared,
};
use super::retention::{RetentionAuthority, RetentionCell};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering as CmpOrdering,
    fmt,
    hash::{Hash, Hasher},
    sync::{
        Arc, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

struct Identifier {
    text: Option<String>,
    lease: Shared<IdentifierLease>,
}
impl Drop for Identifier {
    fn drop(&mut self) {
        drop(self.text.take());
        if let Some(allocation) = &self.lease.payload_allocation {
            allocation.retire_after_physical();
        }
        self.lease.retired.store(true, Ordering::Release);
    }
}
#[derive(Debug)]
struct IdentifierLease {
    retention: OnceLock<RetentionCell>,
    payload_bytes: usize,
    retired: AtomicBool,
    payload_allocation: Option<Reservation>,
    allocation: Option<Reservation>,
}
fn inline_bytes() -> usize {
    super::allocations::shared_inline_bytes::<Identifier>()
        + super::allocations::shared_inline_bytes::<IdentifierLease>()
}
/// Genuine weak identifier header and immutable post-destruction metadata.
#[derive(Clone)]
pub struct IdentifierIdentity {
    _body: Weak<Identifier>,
    lease: Shared<IdentifierLease>,
}
impl IdentifierIdentity {
    pub fn matches_tool(&self, current: &ToolId) -> bool {
        self._body.ptr_eq(&Arc::downgrade(&current.body))
    }
    pub fn retained_heap_bytes(&self) -> Option<usize> {
        inline_bytes().checked_add(if self.lease.retired.load(Ordering::Acquire) {
            0
        } else {
            self.lease.payload_bytes
        })
    }
}
impl std::fmt::Debug for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Identifier")
            .field(&self.text.as_deref())
            .finish()
    }
}
impl crate::model::retention::graph::sealed::Source for IdentifierIdentity {}
impl crate::model::retention::graph::OwnerGraph for IdentifierIdentity {
    fn count(
        &self,
        count: &mut crate::model::retention::graph::Count<'_, '_>,
    ) -> Result<(), crate::model::retention::RetentionError> {
        let payload = if self.lease.retired.load(Ordering::Acquire) {
            0
        } else {
            self.lease.payload_bytes
        };
        let bytes = inline_bytes()
            .checked_add(payload)
            .ok_or(crate::model::retention::RetentionError::Arithmetic)?;
        if count.enter_id(&self.lease.retention, bytes)? {
            for allocation in [&self.lease.allocation, &self.lease.payload_allocation]
                .into_iter()
                .flatten()
            {
                allocation.validate_pending(count)?;
            }
        }
        Ok(())
    }
    fn cleanup(&self, permit: &crate::model::retention::CensusPermit<'_>) {
        if let Some(cell) = self.lease.retention.get()
            && cell.clear_owned(permit)
        {
            for allocation in [&self.lease.allocation, &self.lease.payload_allocation]
                .into_iter()
                .flatten()
            {
                allocation.cleanup_pending(permit);
            }
        }
    }

    fn adopt(&self, publication: &mut crate::model::retention::document::Publication<'_>) {
        if let Some(cell) = self.lease.retention.get()
            && publication.enter(cell)
        {
            for allocation in [&self.lease.allocation, &self.lease.payload_allocation]
                .into_iter()
                .flatten()
            {
                allocation.adopt_under(publication);
            }
        }
    }
}
macro_rules! identifiers {
    ($($name:ident),+ $(,)?) => { $(
        #[derive(Clone, Debug)]
        pub struct $name { body: Arc<Identifier>, lease: Shared<IdentifierLease> }
        impl PartialEq for $name { fn eq(&self,other:&Self)->bool { self.as_str()==other.as_str() } }
        impl Eq for $name {}
        impl PartialOrd for $name { fn partial_cmp(&self,other:&Self)->Option<CmpOrdering> {Some(self.cmp(other))} }
        impl Ord for $name {fn cmp(&self,other:&Self)->CmpOrdering { self.as_str().cmp(other.as_str()) }}
        impl Hash for $name {fn hash<H:Hasher>(&self,state:&mut H) {self.as_str().hash(state);}}
        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self::build(value.into(), None, None, None)
            }
            pub fn supplied(authority: &RetentionAuthority, value: impl Into<String>) -> Self {
                Self::build(value.into(), Some(authority), None, None)
            }
            pub(crate) fn admitted(text: super::native::NativeOwned<String>) -> Result<Self, JsonDecodeError> {
                let ledger = text.ledger().share();
                let allocation = ledger.reserve(Self::inline_bytes())?;
                let (text, payload_allocation) = text.into_parts();
                Ok(Self::build(text, Some(ledger.authority()), Some(allocation), Some(payload_allocation)))
            }
            fn build(text: String, authority: Option<&RetentionAuthority>, allocation: Option<Reservation>, payload_allocation: Option<Reservation>) -> Self {
                let retention = OnceLock::new();
                if let Some(authority) = authority { let _ = retention.set(RetentionCell::new(authority)); }
                let lease = Shared::new(IdentifierLease { retention, payload_bytes:text.capacity(), retired:AtomicBool::new(false), payload_allocation, allocation });
                let body = Arc::new(Identifier { text:Some(text), lease:lease.clone() });
                Self { body, lease }
            }
            pub fn retention_identity(&self) -> IdentifierIdentity {
                IdentifierIdentity { _body:Arc::downgrade(&self.body), lease:self.lease.clone() }
            }
            pub fn inline_bytes() -> usize { inline_bytes() }
            /// Named compatibility intake fixes physical allocation affinity,
            /// independently of later admission success. It never rewrites text
            /// or issues executor authority; a foreign host needs a new owner.
            pub fn adopt_retention(&self, authority:&RetentionAuthority)->Result<(),super::retention::RetentionError> {
                let cell=self.lease.retention.get_or_init(||RetentionCell::new(authority));
                if cell.belongs_to(authority) { Ok(()) } else { Err(super::retention::RetentionError::ForeignAuthority) }
            }
            pub fn as_str(&self) -> &str { self.body.text.as_ref().expect("live identifier").as_str() }
            /// Clones of one supplied identifier share their literal owner.
            pub fn shares_storage(&self, other: &Self) -> bool { Arc::ptr_eq(&self.body, &other.body) }
            /// Retained native heap, including immutable shared owner and actual reserve.
            /// The physical owner, rather than the spelling, carries census identity.
            pub fn retained_heap_bytes(&self) -> Option<usize> {
                Self::inline_bytes().checked_add(self.lease.payload_bytes)
            }
        }
        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.body.text.as_ref().expect("live identifier").as_str())
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
                String::deserialize(decoder).map(Self::new)
            }
        }
        impl crate::model::decode::Decode for $name {
            fn decode<'de,D:serde::Deserializer<'de>>(authority:&RetentionAuthority, decoder:D)->Result<Self,D::Error> {
                String::deserialize(decoder).map(|value| Self::supplied(authority, value))
            }
        }
        impl crate::model::retention::graph::sealed::Source for $name {}
        impl crate::model::retention::graph::OwnerGraph for $name {
            fn count(&self, count:&mut crate::model::retention::graph::Count<'_, '_>)->Result<(),crate::model::retention::RetentionError> {
                self.retention_identity().count(count)
            }
            fn cleanup(&self, permit:&crate::model::retention::CensusPermit<'_>) {
                self.retention_identity().cleanup(permit);
            }

    fn adopt(&self, publication: &mut crate::model::retention::document::Publication<'_>) {
                self.retention_identity().adopt(publication);
    }
}

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { fmt::Display::fmt(self.as_str(), f) }
        }
    )+ };
}
identifiers!(
    SessionId,
    ToolId,
    ModeId,
    ConfigId,
    ConfigChoiceId,
    ConfigGroupId,
    AuthMethodId,
    PermissionOptionId,
    ProviderTerminalId
);
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cloning_supplied_ids_shares_literal_bytes_without_normalization() {
        let original = SessionId::new(" 001 ".repeat(100_000));
        let held = original.clone();
        assert_eq!(original.as_str().as_ptr(), held.as_str().as_ptr());
        assert_eq!(
            serde_json::to_value(&held).unwrap(),
            serde_json::Value::String(original.as_str().into())
        );
    }
    #[test]
    fn owning_a_supplied_string_moves_its_buffer_and_retains_capacity() {
        let mut supplied = String::with_capacity(17 * 1024 * 1024);
        supplied.push_str(" 001 ");
        let pointer = supplied.as_ptr();
        let capacity = supplied.capacity();
        let id = ToolId::new(supplied);
        assert_eq!(id.as_str().as_ptr(), pointer);
        assert!(id.retained_heap_bytes().unwrap() >= capacity);
        assert_eq!(id.as_str(), " 001 ");
        assert_eq!(id.clone().as_str().as_ptr(), pointer);
    }
    #[test]
    fn supplied_identifiers_preserve_bytes_without_inferring_authority() {
        assert_eq!(SessionId::new(" 001 ").as_str(), " 001 ");
        assert_ne!(ToolId::new("001"), ToolId::new("1"));
        let id = ModeId::new("");
        assert_eq!(
            serde_json::from_str::<ModeId>(&serde_json::to_string(&id).unwrap()).unwrap(),
            id
        );
    }
}
