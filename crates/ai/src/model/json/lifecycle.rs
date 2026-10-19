//! Checked literal cleanup and payload tokens stay paired on every ownership path.
//! Untracked/legacy construction without cleanup remains stack-unsafe explicitly.
use super::{
    deferred::{DecodeLedger, JsonDecodeError, ledger::Reservation},
    lifetime::{HeaderLease, Shared},
};
mod native;
mod retention;
pub(super) use native::NativeCursor;
pub use native::NativeText;
pub(super) use retention::CleanupIdentity;
use serde_json::Value;
use std::{fmt, mem, sync::Mutex};

pub(super) struct Workspace {
    work: Mutex<Vec<Value>>,
    frontier: usize,
    allocation: Option<Reservation>,
    identity: CleanupIdentity,
}
pub(super) type Cleanup = Shared<Workspace>;
impl Shared<Workspace> {
    pub(super) fn admit(frontier: usize, ledger: &DecodeLedger) -> Result<Self, JsonDecodeError> {
        if frontier == 0 {
            return Err(JsonDecodeError::CleanupFrontier);
        }
        let bytes = frontier
            .checked_mul(mem::size_of::<Value>())
            .ok_or(JsonDecodeError::Arithmetic)?;
        let mut allocation = ledger.reserve(bytes)?;
        allocation.grow(crate::model::allocations::shared_inline_bytes::<Workspace>())?;
        let workspace_allocation = allocation.split(allocation.bytes())?;
        let identity = CleanupIdentity::admit(ledger, workspace_allocation)?;
        let mut work = Vec::new();
        work.try_reserve_exact(frontier)?;
        if work.capacity() != frontier {
            return Err(JsonDecodeError::CapacityProfile);
        }
        Ok(Self::new(Workspace {
            work: Mutex::new(work),
            frontier,
            allocation: Some(allocation),
            identity,
        }))
    }
    /// Private frontier certificates guarantee existing capacity for every push.
    /// Scratch holds raw Values only: no callbacks, guarded nodes or nested locks.
    fn drain(&self, root: Value) {
        let mut work = self
            .get()
            .work
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        work.push(root);
        while let Some(value) = work.pop() {
            match value {
                Value::Array(values) => {
                    for value in values {
                        work.push(value);
                    }
                }
                Value::Object(values) => {
                    for (_, value) in values {
                        work.push(value);
                    }
                }
                _ => {}
            }
        }
    }
    pub(super) fn check(&self, frontier: usize) -> Result<(), JsonDecodeError> {
        if frontier > self.get().frontier {
            return Err(JsonDecodeError::CleanupFrontier);
        }
        Ok(())
    }
    fn belongs_to(&self, ledger: &DecodeLedger) -> bool {
        self.get()
            .allocation
            .as_ref()
            .is_some_and(|allocation| allocation.belongs_to(ledger))
    }
    fn identity(&self) -> &CleanupIdentity {
        &self.get().identity
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        // Shared::drop already freed this workspace's Arc block. No guarded nodes
        // or in-flight workspace borrowers can survive its last private handle.
        let work = self
            .work
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        drop(mem::take(work));
        drop(self.allocation.take());
        self.identity.retire();
    }
}
/// Raw payload and its authentic token never enter independent fallible locals.
pub(super) struct GuardedValue {
    value: Value,
    cleanup: Option<Cleanup>,
    frontier: usize,
    allocation: Reservation,
}
impl GuardedValue {
    pub(super) fn native_value(&self) -> &Value {
        &self.value
    }
    pub(super) fn native_cleanup(&self) -> Option<Cleanup> {
        self.cleanup.clone()
    }
    pub(super) fn take_payload_allocation(&mut self) -> Result<Reservation, JsonDecodeError> {
        self.allocation.split(self.allocation.bytes())
    }
    pub(super) fn exclusive_bytes(&self) -> usize {
        self.allocation.bytes()
    }
    pub(super) fn cleanup_identity(&self) -> Option<CleanupIdentity> {
        self.cleanup
            .as_ref()
            .map(|cleanup| cleanup.identity().clone())
    }

    pub(super) fn new(
        value: Value,
        frontier: usize,
        cleanup: Option<Cleanup>,
        allocation: Reservation,
    ) -> Self {
        Self {
            value,
            cleanup,
            frontier,
            allocation,
        }
    }
    pub(super) fn reparent(mut self, cleanup: &Cleanup) -> Result<Self, JsonDecodeError> {
        if !cleanup.belongs_to(self.allocation.original_ledger()) {
            return Err(JsonDecodeError::ForeignLedger);
        }
        cleanup.check(self.frontier)?;
        self.cleanup = Some(cleanup.clone());
        Ok(self)
    }
    pub(super) fn frontier(&self) -> usize {
        self.frontier
    }
    pub(super) fn belongs_to(&self, ledger: &DecodeLedger) -> bool {
        self.allocation.belongs_to(ledger)
            && self
                .cleanup
                .as_ref()
                .is_none_or(|cleanup| cleanup.belongs_to(ledger))
    }
    fn same_cleanup(&self, child: &Self) -> bool {
        match (&self.cleanup, &child.cleanup) {
            (Some(a), Some(b)) => a.ptr_eq(b),
            (None, None) => true,
            _ => false,
        }
    }
    pub(super) fn push_array(&mut self, mut child: Self) -> Result<(), JsonDecodeError> {
        if !self.same_cleanup(&child) {
            return Err(JsonDecodeError::CleanupFrontier);
        }
        let Value::Array(values) = &mut self.value else {
            return Err(JsonDecodeError::ConstructionOrder);
        };
        self.allocation.merge_from(&mut child.allocation)?;
        values.push(mem::take(&mut child.value));
        Ok(())
    }
    pub(super) fn insert_object(
        &mut self,
        key: String,
        mut child: Self,
    ) -> Result<(), JsonDecodeError> {
        if !self.same_cleanup(&child) {
            return Err(JsonDecodeError::CleanupFrontier);
        }
        let Value::Object(values) = &mut self.value else {
            return Err(JsonDecodeError::ConstructionOrder);
        };
        if values.contains_key(&key) {
            return Err(JsonDecodeError::ConstructionOrder);
        }
        self.allocation.merge_from(&mut child.allocation)?;
        values.insert(key, mem::take(&mut child.value));
        Ok(())
    }
}
impl Drop for GuardedValue {
    fn drop(&mut self) {
        if matches!(self.value, Value::Array(_) | Value::Object(_))
            && let Some(cleanup) = &self.cleanup
        {
            cleanup.drain(mem::take(&mut self.value));
        }
        // Automatic fields drop in declaration order: raw Value, cleanup handle,
        // then authentic payload token. Uncontrolled Value drop is still recursive.
    }
}
enum Root {
    Admitted(GuardedValue),
    Untracked(Value),
}
pub(super) struct Backing {
    value: Option<Root>,
    lease: HeaderLease,
}
impl Backing {
    pub(super) fn untracked(value: Value, lease: HeaderLease) -> Self {
        Self {
            value: Some(Root::Untracked(value)),
            lease,
        }
    }
    pub(super) fn admitted(value: GuardedValue, lease: HeaderLease) -> Self {
        Self {
            value: Some(Root::Admitted(value)),
            lease,
        }
    }
    pub(super) fn value(&self) -> &Value {
        match self.value.as_ref().expect("live immutable JSON backing") {
            Root::Admitted(value) => &value.value,
            Root::Untracked(value) => value,
        }
    }
    pub(super) fn belongs_to(&self, ledger: &DecodeLedger) -> bool {
        self.lease.belongs_to(ledger)
            && self.value.as_ref().is_some_and(|value| match value {
                Root::Admitted(value) => value.belongs_to(ledger),
                Root::Untracked(_) => false,
            })
    }
}
impl Drop for Backing {
    fn drop(&mut self) {
        // Drop completes real JSON/workspace destruction and token retirement
        // BEFORE Release publication. strong_count==0 alone is not this proof.
        drop(self.value.take());
        self.lease.retire_payload();
    }
}
impl fmt::Debug for Backing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JsonBacking").finish_non_exhaustive()
    }
}
#[cfg(test)]
mod tests;
