//! Genuine per-allocation liabilities; topology has weak account linkage only.
use super::{DocumentRetention, RetentionError, phase::PhaseIdentity};
use std::{
    mem::size_of,
    sync::atomic::{AtomicU8, Ordering},
};

/// A request derived immediately by the private real allocator, never evidence
/// of existing memory. No constructor is exposed to adapters or feature crates.
pub(crate) struct AllocationPlan {
    bytes: usize,
}
impl AllocationPlan {
    pub(crate) fn array<T>(capacity: usize) -> Result<Self, RetentionError> {
        Self::checked_bytes(
            capacity
                .checked_mul(size_of::<T>())
                .ok_or(RetentionError::Arithmetic)?,
        )
    }
    pub(crate) fn text(capacity: usize) -> Self {
        Self { bytes: capacity }
    }
    pub(crate) fn shared<T>() -> Self {
        Self {
            bytes: crate::model::allocations::shared_inline_bytes::<T>(),
        }
    }
    pub(crate) fn checked_bytes(bytes: usize) -> Result<Self, RetentionError> {
        if bytes > isize::MAX as usize {
            return Err(RetentionError::Arithmetic);
        }
        Ok(Self { bytes })
    }
    pub(super) fn bytes(&self) -> usize {
        self.bytes
    }
}
/// Held only inside the closed publication transition, never supplied to caller
/// callbacks. The genuine sealed owner graph visits its own allocation tokens.
pub(crate) struct Publication<'a> {
    scope: &'a DocumentRetention,
    pending: &'a mut usize,
    permit: crate::model::retention::CensusPermit<'a>,
}
impl<'a> Publication<'a> {
    pub(crate) fn new(
        scope: &'a DocumentRetention,
        pending: &'a mut usize,
    ) -> Result<Self, RetentionError> {
        let mut permit = scope.authority().permit();
        permit.begin()?;
        Ok(Self {
            scope,
            pending,
            permit,
        })
    }
    pub(crate) fn enter(&self, cell: &crate::model::retention::RetentionCell) -> bool {
        cell.first(&self.permit)
            .expect("validated adoption authority")
    }
    pub(crate) fn permit(&self) -> &crate::model::retention::CensusPermit<'a> {
        &self.permit
    }
}
/// Actual native/staging lifetime envelopes strongly pin the scope; tokens,
/// ledger and projection metadata never do, preventing a registration cycle.
#[must_use]
#[derive(Debug)]
pub(crate) struct PendingAllocation {
    account: crate::model::json::lifetime::Shared<super::account::AccountIdentity>,
    bytes: usize,
    phase: Option<PhaseIdentity>,
    state: AtomicU8,
}
impl PendingAllocation {
    pub(super) fn new(
        scope: &DocumentRetention,
        plan: AllocationPlan,
        phase: Option<PhaseIdentity>,
    ) -> Self {
        Self {
            account: scope.1.clone(),
            bytes: plan.bytes,
            phase,
            state: AtomicU8::new(PENDING),
        }
    }
    pub(super) fn metadata(scope: &DocumentRetention, bytes: usize) -> Self {
        Self {
            account: scope.1.clone(),
            bytes,
            phase: None,
            state: AtomicU8::new(PENDING),
        }
    }
    pub(crate) fn belongs_to(&self, scope: &DocumentRetention) -> bool {
        self.account.same(&scope.1)
    }
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }
    pub(crate) fn is_pending(&self) -> bool {
        self.state.load(Ordering::Relaxed) == PENDING
    }
    pub(crate) fn validate_for_count(
        &self,
        count: &mut crate::model::retention::graph::Count<'_, '_>,
    ) -> Result<(), RetentionError> {
        count.validate_pending(self)?;
        count.record_adoptable(self)?;
        crate::model::retention::graph::OwnerGraph::count(&self.account, count)?;
        if let Some(phase) = &self.phase {
            crate::model::retention::graph::OwnerGraph::count(phase, count)?;
        }
        Ok(())
    }
    /// Only actual fixed endpoint headers keep phase-independent payment after
    /// publication. A named fork counts foreign headers physically while their
    /// original account still owns their payment; no payload affinity is relaxed.
    pub(crate) fn validate_endpoint_header_for_count(
        &self,
        count: &mut crate::model::retention::graph::Count<'_, '_>,
    ) -> Result<(), RetentionError> {
        if self.phase.is_some() {
            return Err(RetentionError::WrongPayload);
        }
        crate::model::retention::graph::OwnerGraph::count(&self.account, count)
    }
    pub(crate) fn cleanup_for_count(&self, permit: &crate::model::retention::CensusPermit<'_>) {
        crate::model::retention::graph::OwnerGraph::cleanup(&self.account, permit);
        if let Some(phase) = &self.phase {
            crate::model::retention::graph::OwnerGraph::cleanup(phase, permit);
        }
    }
    pub(crate) fn merge_from(&mut self, other: &mut Self) -> Result<(), RetentionError> {
        if !self.account.same(&other.account) || !same_phase(&self.phase, &other.phase) {
            return Err(RetentionError::ForeignAuthority);
        }
        if self.state.load(Ordering::Relaxed) != PENDING
            || other.state.load(Ordering::Relaxed) != PENDING
        {
            return Err(RetentionError::FrozenOwner);
        }
        let total = self
            .bytes
            .checked_add(other.bytes)
            .ok_or(RetentionError::Arithmetic)?;
        self.bytes = total;
        other.bytes = 0;
        Ok(())
    }
    pub(crate) fn split(&mut self, bytes: usize) -> Result<Self, RetentionError> {
        if self.state.load(Ordering::Relaxed) != PENDING {
            return Err(RetentionError::FrozenOwner);
        }
        let remaining = self
            .bytes
            .checked_sub(bytes)
            .ok_or(RetentionError::Arithmetic)?;
        self.bytes = remaining;
        Ok(Self {
            account: self.account.clone(),
            bytes,
            phase: self.phase.clone(),
            state: AtomicU8::new(PENDING),
        })
    }
    pub(crate) fn adopt_under(&self, publication: &mut Publication<'_>) {
        crate::model::retention::graph::OwnerGraph::adopt(&self.account, publication);
        if let Some(phase) = &self.phase {
            crate::model::retention::graph::OwnerGraph::adopt(phase, publication);
        }
        if !self.is_pending() {
            return;
        }
        assert!(
            self.belongs_to(publication.scope),
            "actual pending allocation scope"
        );
        if self
            .state
            .compare_exchange(PENDING, ADOPTED, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            *publication.pending = publication
                .pending
                .checked_sub(self.bytes)
                .expect("authentic adoption credit");
        }
    }
    /// Only the actual typed backing owner invokes this after physical free.
    /// Immutable original bytes avoid a retire/adopt zeroing race; exactly the
    /// winning Pending transition removes its original liability.
    pub(crate) fn retire_after_physical(&self) {
        if self.state.swap(RETIRED, Ordering::Relaxed) == PENDING
            && self.bytes != 0
            && let Some(account) = super::account::AccountIdentity::upgrade(&self.account)
        {
            account.refund(self.bytes);
        }
    }
}
impl Drop for PendingAllocation {
    fn drop(&mut self) {
        self.retire_after_physical();
    }
}
const PENDING: u8 = 0;
const ADOPTED: u8 = 1;
const RETIRED: u8 = 2;

fn same_phase(left: &Option<PhaseIdentity>, right: &Option<PhaseIdentity>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.same(right),
        (None, None) => true,
        _ => false,
    }
}
