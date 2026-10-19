//! Genuine operation incarnations. Weak keys cannot allocate after their actual
//! producer closes. Header credit survives all real Weak handles.
use super::{AllocationPlan, DocumentRetention, PendingAllocation, RetentionError};
use crate::model::json::lifetime::Shared;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct Phase {
    active: AtomicBool,
    account: Shared<super::account::AccountIdentity>,
}
struct PhaseLifetime {
    // This real Weak drops after the actual strong body. Shared::into_inner
    // physically frees this lease block before these fields retire in order.
    _header: Weak<Phase>,
    credit: PendingAllocation,
    cell: crate::model::retention::RetentionCell,
}
pub(super) struct PhaseOwner {
    body: Arc<Phase>,
    lifetime: Shared<PhaseLifetime>,
}
#[derive(Clone)]
pub(super) struct PhaseIdentity {
    header: Weak<Phase>,
    _lifetime: Shared<PhaseLifetime>,
}
impl PhaseIdentity {
    pub(super) fn same(&self, other: &Self) -> bool {
        self.header.ptr_eq(&other.header)
    }
    pub(super) fn upgrade(&self) -> Result<Arc<Phase>, RetentionError> {
        self.header.upgrade().ok_or(RetentionError::FrozenOwner)
    }
}
impl Phase {
    pub(super) fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }
    pub(super) fn close(&self) {
        self.active.store(false, Ordering::Relaxed);
    }
}
impl PhaseOwner {
    pub(super) fn new(scope: &DocumentRetention) -> Result<Self, RetentionError> {
        let bytes = crate::model::allocations::shared_inline_bytes::<Phase>()
            .checked_add(crate::model::allocations::shared_inline_bytes::<
                PhaseLifetime,
            >())
            .ok_or(RetentionError::Arithmetic)?;
        let credit = scope.reserve_phase_metadata(AllocationPlan::checked_bytes(bytes)?)?;
        let body = Arc::new(Phase {
            active: AtomicBool::new(true),
            account: scope.1.clone(),
        });
        let lifetime = Shared::new(PhaseLifetime {
            _header: Arc::downgrade(&body),
            credit,
            cell: crate::model::retention::RetentionCell::new(scope.authority()),
        });
        Ok(Self { body, lifetime })
    }
    pub(super) fn identity(&self) -> PhaseIdentity {
        PhaseIdentity {
            header: Arc::downgrade(&self.body),
            _lifetime: self.lifetime.clone(),
        }
    }
    pub(super) fn key(&self) -> ConstructionKey {
        ConstructionKey {
            identity: self.identity(),
        }
    }
    pub(super) fn close(&self, scope: &DocumentRetention) {
        if let Ok(_state) = scope.state() {
            self.body.close();
        }
    }
    pub(super) fn active(&self) -> bool {
        self.body.is_active()
    }
}
/// Only ConstructionRole mints this genuine allocation key. It holds no scope
/// strongly and no allocation or I/O lock over a native operation.
#[derive(Clone)]
pub(crate) struct ConstructionKey {
    identity: PhaseIdentity,
}
impl ConstructionKey {
    pub(crate) fn reserve_policy(
        &self,
        bytes: usize,
    ) -> Result<super::PolicyLiability, RetentionError> {
        let phase = self.identity.upgrade()?;
        let scope = super::account::AccountIdentity::upgrade(&phase.account)
            .ok_or(RetentionError::FrozenOwner)?;
        scope.reserve_semantic(bytes, &self.identity)
    }
    pub(crate) fn reserve_pending(
        &self,
        plan: AllocationPlan,
    ) -> Result<PendingAllocation, RetentionError> {
        let phase = self.identity.upgrade()?;
        let scope = super::account::AccountIdentity::upgrade(&phase.account)
            .ok_or(RetentionError::FrozenOwner)?;
        scope.reserve_for_phase(plan, &self.identity)
    }
    pub(crate) fn belongs_to(&self, scope: &DocumentRetention) -> bool {
        self.identity
            .upgrade()
            .is_ok_and(|phase| phase.account.same(&scope.1))
    }
    pub(crate) fn validate_live(&self) -> Result<(), RetentionError> {
        if self.identity.upgrade()?.is_active() {
            Ok(())
        } else {
            Err(RetentionError::FrozenOwner)
        }
    }
}
impl std::fmt::Debug for ConstructionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConstructionKey").finish_non_exhaustive()
    }
}
impl std::fmt::Debug for PhaseIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhaseIdentity").finish_non_exhaustive()
    }
}

impl crate::model::retention::graph::sealed::Source for PhaseIdentity {}
impl crate::model::retention::graph::OwnerGraph for PhaseIdentity {
    fn count(
        &self,
        count: &mut crate::model::retention::graph::Count<'_, '_>,
    ) -> Result<(), RetentionError> {
        let bytes = crate::model::allocations::shared_inline_bytes::<Phase>()
            .checked_add(crate::model::allocations::shared_inline_bytes::<
                PhaseLifetime,
            >())
            .ok_or(RetentionError::Arithmetic)?;
        let bytes = if count.pays_pending_header(&self._lifetime.credit) {
            0
        } else {
            bytes
        };
        if count.enter(&self._lifetime.cell, bytes)? {
            self._lifetime
                .credit
                .validate_endpoint_header_for_count(count)?;
        }
        Ok(())
    }
    fn cleanup(&self, permit: &crate::model::retention::CensusPermit<'_>) {
        if self._lifetime.cell.clear_owned(permit) {
            self._lifetime.credit.cleanup_for_count(permit);
        }
    }
    fn adopt(&self, publication: &mut super::Publication<'_>) {
        // This fixed header remains paid until the last genuine Weak/lease dies.
        // Publication still marks it for sealed cleanup, without transferring payment.
        publication.enter(&self._lifetime.cell);
    }
}
impl crate::model::retention::graph::sealed::Source for ConstructionKey {}
impl crate::model::retention::graph::OwnerGraph for ConstructionKey {
    fn count(
        &self,
        count: &mut crate::model::retention::graph::Count<'_, '_>,
    ) -> Result<(), RetentionError> {
        crate::model::retention::graph::OwnerGraph::count(&self.identity, count)
    }
    fn cleanup(&self, permit: &crate::model::retention::CensusPermit<'_>) {
        crate::model::retention::graph::OwnerGraph::cleanup(&self.identity, permit);
    }
    fn adopt(&self, publication: &mut super::Publication<'_>) {
        crate::model::retention::graph::OwnerGraph::adopt(&self.identity, publication);
    }
}

#[cfg(test)]
mod tests;
