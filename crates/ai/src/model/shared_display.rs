//! Immutable display ownership and weak provenance share authentic lifetime metadata.
//! Weak provenance keeps headers/JSON receipts, never resurrects display payloads.
use super::json::lifetime::Shared;
use super::{
    json::{
        JsonReceipt,
        deferred::{JsonDecodeError, ledger::Reservation},
    },
    retention::{
        CensusPermit, RetentionCell, RetentionError,
        graph::{Count, OwnerGraph, sealed},
    },
    tools::{ClientDisplay, DisplayOutcome},
};
use std::{
    fmt,
    ops::Deref,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

struct Lease {
    retention: RetentionCell,
    retired: AtomicBool,
    payload_bytes: usize,
    arguments: JsonReceipt,
    outcome: Option<JsonReceipt>,
    payload_allocation: Option<Reservation>,
    allocation: Option<Reservation>,
}
struct Body {
    value: Option<ClientDisplay>,
    lease: Shared<Lease>,
}
impl Drop for Body {
    fn drop(&mut self) {
        drop(self.value.take());
        if let Some(allocation) = &self.lease.payload_allocation {
            allocation.retire_after_physical();
        }
        self.lease.retired.store(true, Ordering::Release);
    }
}
/// A strong immutable presentation value, not an execution capability.
#[derive(Clone)]
pub struct SharedDisplay {
    body: Arc<Body>,
    lease: Shared<Lease>,
}
/// Original display identity remains meaningful after its strong payload retires.
#[derive(Clone)]
pub struct DisplayIdentity {
    body: Weak<Body>,
    lease: Shared<Lease>,
}
impl SharedDisplay {
    /// Compatibility intake owns the already supplied payload. Native controlled
    /// loading uses admitted(), which reserves its actual headers before allocation.
    pub fn from_untracked(value: ClientDisplay) -> Self {
        Self::build(value, None, None)
    }
    pub(crate) fn admitted(
        value: super::native::NativeOwned<ClientDisplay>,
    ) -> Result<Self, JsonDecodeError> {
        let allocation = value.ledger().reserve(Self::inline_bytes())?;
        let (value, payload_allocation) = value.into_parts();
        Ok(Self::build(
            value,
            Some(allocation),
            Some(payload_allocation),
        ))
    }
    fn build(
        value: ClientDisplay,
        allocation: Option<Reservation>,
        payload_allocation: Option<Reservation>,
    ) -> Self {
        let payload_bytes = value
            .tool
            .capacity()
            .saturating_add(value.destination.value().map_or(0, String::capacity))
            .saturating_add(match value.outcome.value() {
                Some(DisplayOutcome::Failed(text)) => text.capacity(),
                _ => 0,
            });
        let outcome = match value.outcome.value() {
            Some(DisplayOutcome::Succeeded(json)) => Some(json.receipt().clone()),
            _ => None,
        };
        let lease = Shared::new(Lease {
            retention: RetentionCell::new(value.arguments.authority()),
            retired: AtomicBool::new(false),
            payload_bytes,
            arguments: value.arguments.receipt().clone(),
            outcome,
            payload_allocation,
            allocation,
        });
        let body = Arc::new(Body {
            value: Some(value),
            lease: lease.clone(),
        });
        Self { body, lease }
    }
    pub fn inline_bytes() -> usize {
        super::allocations::shared_inline_bytes::<Body>()
            + super::allocations::shared_inline_bytes::<Lease>()
    }
    pub fn identity(&self) -> DisplayIdentity {
        DisplayIdentity {
            body: Arc::downgrade(&self.body),
            lease: self.lease.clone(),
        }
    }
    pub fn shares_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.body, &other.body)
    }
}
impl DisplayIdentity {
    pub fn matches(&self, current: &SharedDisplay) -> bool {
        self.body.ptr_eq(&Arc::downgrade(&current.body))
    }
    pub fn inline_bytes(&self) -> usize {
        SharedDisplay::inline_bytes()
    }
}
impl Deref for SharedDisplay {
    type Target = ClientDisplay;
    fn deref(&self) -> &Self::Target {
        self.body.value.as_ref().expect("live immutable display")
    }
}
impl fmt::Debug for SharedDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.deref(), f)
    }
}
impl PartialEq for SharedDisplay {
    fn eq(&self, other: &Self) -> bool {
        self.deref() == other.deref()
    }
}
impl serde::Serialize for SharedDisplay {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.deref().serialize(serializer)
    }
}
impl super::decode::Decode for SharedDisplay {
    fn decode<'de, D: serde::Deserializer<'de>>(
        authority: &super::retention::RetentionAuthority,
        decoder: D,
    ) -> Result<Self, D::Error> {
        ClientDisplay::decode(authority, decoder).map(Self::from_untracked)
    }
}
impl sealed::Source for DisplayIdentity {}
impl OwnerGraph for DisplayIdentity {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.lease.count(count)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.lease.cleanup(permit);
    }

    fn adopt(&self, publication: &mut crate::model::retention::document::Publication<'_>) {
        self.lease.adopt(publication);
    }
}
impl sealed::Source for SharedDisplay {}
impl OwnerGraph for SharedDisplay {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.lease.count(count)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.lease.cleanup(permit);
    }

    fn adopt(&self, publication: &mut crate::model::retention::document::Publication<'_>) {
        self.lease.adopt(publication);
    }
}
impl sealed::Source for Lease {}
impl OwnerGraph for Lease {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        let payload = if self.retired.load(Ordering::Acquire) {
            0
        } else {
            self.payload_bytes
        };
        let bytes = SharedDisplay::inline_bytes()
            .checked_add(payload)
            .ok_or(RetentionError::Arithmetic)?;
        if count.enter(&self.retention, bytes)? {
            self.arguments.count(count)?;
            if let Some(outcome) = &self.outcome {
                outcome.count(count)?;
            }
            for allocation in [&self.allocation, &self.payload_allocation]
                .into_iter()
                .flatten()
            {
                allocation.validate_pending(count)?;
            }
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        if self.retention.clear_owned(permit) {
            self.arguments.cleanup(permit);
            if let Some(outcome) = &self.outcome {
                outcome.cleanup(permit);
            }
            for allocation in [&self.allocation, &self.payload_allocation]
                .into_iter()
                .flatten()
            {
                allocation.cleanup_pending(permit);
            }
        }
    }

    fn adopt(&self, publication: &mut crate::model::retention::document::Publication<'_>) {
        if publication.enter(&self.retention) {
            self.arguments.adopt(publication);
            if let Some(outcome) = &self.outcome {
                outcome.adopt(publication);
            }
            for allocation in [&self.allocation, &self.payload_allocation]
                .into_iter()
                .flatten()
            {
                allocation.adopt_under(publication);
            }
        }
    }
}
