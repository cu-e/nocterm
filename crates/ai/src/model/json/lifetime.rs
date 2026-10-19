//! Safe shared-allocation retirement after its actual Arc block is deallocated.
use super::{
    deferred::{DecodeLedger, JsonDecodeError, ledger::Reservation},
    lifecycle::{Backing, GuardedValue},
};
use crate::model::retention::{RetentionAuthority, RetentionCell};
use std::{
    fmt,
    sync::{
        Arc, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

/// No raw Arc/Weak escapes. Every owner uses into_inner: exactly one concurrent
/// last caller obtains the value AFTER Arc's implicit weak/block has retired.
pub(crate) struct Shared<T>(Option<Arc<T>>);
impl<T> Shared<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(Some(Arc::new(value)))
    }
    pub(crate) fn get_mut(&mut self) -> Option<&mut T> {
        self.0.as_mut().and_then(Arc::get_mut)
    }
    pub(crate) fn get(&self) -> &T {
        self.0.as_deref().expect("live private allocation handle")
    }
    pub(super) fn ptr_eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}
impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(self.0.as_ref().map(Arc::clone))
    }
}
impl<T> Drop for Shared<T> {
    fn drop(&mut self) {
        if let Some(value) = self.0.take().and_then(Arc::into_inner) {
            drop(value);
        }
    }
}
impl<T> fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedAllocation").finish_non_exhaustive()
    }
}
/// The Weak header retires before its backing reservation. The containing Shared
/// has already physically freed the lease block before any of these fields drop.
struct RootBinding {
    _backing: Weak<Backing>,
    payload_bytes: usize,
    payload_retired: AtomicBool,
    payload_allocation: Option<Reservation>,
    _allocation: Option<Reservation>,
}
enum LeaseRole {
    Untracked,
    Anchor {
        workspace_bytes: usize,
        workspace_allocation: Reservation,
        retired: AtomicBool,
        claimed: AtomicBool,
    },
    Child(HeaderLease),
}
pub(super) struct IdentityLease {
    root: OnceLock<RootBinding>,
    role: LeaseRole,
    retention: RetentionCell,
    // Last: root Weak and shared cleanup links physically retire before credit.
    allocation: Option<Reservation>,
}
impl IdentityLease {
    fn allocations(&self) -> [Option<&Reservation>; 4] {
        let root = self.root.get();
        [
            self.allocation.as_ref(),
            root.and_then(|root| root._allocation.as_ref()),
            root.and_then(|root| root.payload_allocation.as_ref()),
            match &self.role {
                LeaseRole::Anchor {
                    workspace_allocation,
                    ..
                } => Some(workspace_allocation),
                _ => None,
            },
        ]
    }
}
pub(super) type HeaderLease = Shared<IdentityLease>;
impl HeaderLease {
    pub(super) fn admit(
        ledger: &DecodeLedger,
        workspace_allocation: Reservation,
    ) -> Result<Self, JsonDecodeError> {
        let allocation = ledger.reserve(header_bytes())?;
        Ok(Self::new(IdentityLease {
            root: OnceLock::new(),
            role: LeaseRole::Anchor {
                workspace_bytes: workspace_allocation.bytes(),
                workspace_allocation,
                retired: AtomicBool::new(false),
                claimed: AtomicBool::new(false),
            },
            retention: RetentionCell::new(ledger.authority()),
            allocation: Some(allocation),
        }))
    }
    pub(super) fn admitted(
        ledger: &DecodeLedger,
        value: &GuardedValue,
    ) -> Result<Self, JsonDecodeError> {
        if let Some(anchor) = value.cleanup_identity() {
            if let LeaseRole::Anchor { claimed, .. } = &anchor.get().role
                && !claimed.swap(true, Ordering::AcqRel)
            {
                return Ok(anchor);
            }
            return Self::child(ledger, anchor);
        }
        Self::childless(ledger)
    }
    fn child(ledger: &DecodeLedger, anchor: HeaderLease) -> Result<Self, JsonDecodeError> {
        Self::tracked(ledger, LeaseRole::Child(anchor))
    }
    fn childless(ledger: &DecodeLedger) -> Result<Self, JsonDecodeError> {
        Self::tracked(ledger, LeaseRole::Untracked)
    }
    fn tracked(ledger: &DecodeLedger, role: LeaseRole) -> Result<Self, JsonDecodeError> {
        let allocation = ledger.reserve(header_bytes())?;
        Ok(Self::new(IdentityLease {
            root: OnceLock::new(),
            role,
            retention: RetentionCell::new(ledger.authority()),
            allocation: Some(allocation),
        }))
    }
    pub(super) fn untracked(authority: &RetentionAuthority, _payload_bytes: usize) -> Self {
        Self::new(IdentityLease {
            root: OnceLock::new(),
            role: LeaseRole::Untracked,
            retention: RetentionCell::new(authority),
            allocation: None,
        })
    }
    /// Called once before the genuine root/receipt is published to any caller.
    pub(super) fn bind(
        &self,
        backing: &Arc<Backing>,
        payload_bytes: usize,
        allocation: Option<Reservation>,
        payload_allocation: Option<Reservation>,
    ) {
        let binding = RootBinding {
            _backing: Arc::downgrade(backing),
            payload_bytes,
            payload_retired: AtomicBool::new(false),
            payload_allocation,
            _allocation: allocation,
        };
        assert!(
            self.get().root.set(binding).is_ok(),
            "fresh JSON root binding"
        );
    }
    pub(super) fn retired_bytes(&self) -> Option<usize> {
        self.retained_root_bytes()
    }
    pub(super) fn retained_root_bytes(&self) -> Option<usize> {
        let owner = self.get();
        let root = owner.root.get();
        let mut bytes = header_bytes();
        if let Some(root) = root {
            bytes = bytes.checked_add(backing_bytes())?;
            if !root.payload_retired.load(Ordering::Acquire) {
                bytes = bytes.checked_add(root.payload_bytes)?;
            }
        }
        match &owner.role {
            LeaseRole::Anchor {
                workspace_bytes,
                retired,
                ..
            } => {
                if !retired.load(Ordering::Acquire) {
                    bytes = bytes.checked_add(*workspace_bytes)?;
                }
            }
            LeaseRole::Child(anchor) => {
                bytes = bytes.checked_add(anchor.retained_root_bytes()?)?;
            }
            LeaseRole::Untracked => {}
        }
        Some(bytes)
    }
    pub(super) fn retire(&self) {
        if let LeaseRole::Anchor {
            retired,
            workspace_allocation,
            ..
        } = &self.get().role
        {
            workspace_allocation.retire_after_physical();
            retired.store(true, Ordering::Release);
        }
    }
    pub(super) fn authority(&self) -> &RetentionAuthority {
        self.get().retention.authority()
    }
    pub(super) fn retire_payload(&self) {
        let root = self.get().root.get().expect("published root");
        if let Some(allocation) = &root.payload_allocation {
            allocation.retire_after_physical();
        }
        root.payload_retired.store(true, Ordering::Release);
    }
    pub(super) fn payload_retired(&self) -> bool {
        self.get()
            .root
            .get()
            .is_some_and(|root| root.payload_retired.load(Ordering::Acquire))
    }
    pub(super) fn original_ledger(&self) -> Option<&DecodeLedger> {
        self.get()
            .allocation
            .as_ref()
            .map(Reservation::original_ledger)
    }
    pub(super) fn belongs_to(&self, ledger: &DecodeLedger) -> bool {
        self.get()
            .allocation
            .as_ref()
            .is_some_and(|allocation| allocation.belongs_to(ledger))
    }
}
pub(super) fn backing_bytes() -> usize {
    crate::model::allocations::shared_inline_bytes::<Backing>()
}
fn header_bytes() -> usize {
    crate::model::allocations::shared_inline_bytes::<IdentityLease>()
}
pub(super) fn identity_bytes() -> Option<usize> {
    header_bytes().checked_add(backing_bytes())
}
#[cfg(test)]
mod tests;

impl crate::model::retention::graph::sealed::Source for HeaderLease {}
impl crate::model::retention::graph::OwnerGraph for HeaderLease {
    fn count(
        &self,
        count: &mut crate::model::retention::graph::Count<'_, '_>,
    ) -> Result<(), crate::model::retention::RetentionError> {
        use crate::model::retention::RetentionError;
        let owner = self.get();
        let mut bytes = header_bytes();
        if let Some(root) = owner.root.get() {
            bytes = bytes
                .checked_add(backing_bytes())
                .ok_or(RetentionError::Arithmetic)?;
            if !root.payload_retired.load(Ordering::Acquire) {
                bytes = bytes
                    .checked_add(root.payload_bytes)
                    .ok_or(RetentionError::Arithmetic)?;
            }
        }
        if let LeaseRole::Anchor {
            workspace_bytes,
            retired,
            ..
        } = &owner.role
            && !retired.load(Ordering::Acquire)
        {
            bytes = bytes
                .checked_add(*workspace_bytes)
                .ok_or(RetentionError::Arithmetic)?;
        }
        if count.enter(&owner.retention, bytes)? {
            if let LeaseRole::Child(anchor) = &owner.role {
                anchor.count(count)?;
            }
            for allocation in owner.allocations().into_iter().flatten() {
                allocation.validate_pending(count)?;
            }
        }
        Ok(())
    }
    fn cleanup(&self, permit: &crate::model::retention::CensusPermit<'_>) {
        let owner = self.get();
        if owner.retention.clear_owned(permit) {
            if let LeaseRole::Child(anchor) = &owner.role {
                anchor.cleanup(permit);
            }
            for allocation in owner.allocations().into_iter().flatten() {
                allocation.cleanup_pending(permit);
            }
        }
    }

    fn adopt(&self, publication: &mut crate::model::retention::document::Publication<'_>) {
        let owner = self.get();
        if publication.enter(&owner.retention) {
            if let LeaseRole::Child(anchor) = &owner.role {
                anchor.adopt(publication);
            }
            for allocation in owner.allocations().into_iter().flatten() {
                allocation.adopt_under(publication);
            }
        }
    }
}
impl<T> std::ops::Deref for Shared<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.get()
    }
}
