//! Per-document admission across actual producer lifetimes.
//!
//! Owns: ten fixed role capabilities, pending allocation liabilities and the
//! short document -> authority lock order. Does not own scheduling or I/O.
//! Physical projections contain no document reference or payload-lifetime token.
mod account;
mod policy;
pub(crate) use policy::PolicyLiability;
mod credit;
mod payload;
mod phase;
use payload::{RetainedPayload, RetainedProjection};
mod phase_slots;
mod roles;
mod view;
use super::{
    CensusPermit, RetentionAuthority, RetentionError,
    graph::{self, Count, OwnerGraph, sealed},
};
use crate::history::physical::{SnapshotOwner, SnapshotProjection, SnapshotStaging};
pub(crate) use credit::{AllocationPlan, PendingAllocation, Publication};
pub(crate) use phase::ConstructionKey;
use roles::{ConstructionKind, RetainedKind, RetainedRoles};
pub use roles::{ConstructionRole, DocumentRoles, RetainedRole};
use std::sync::{Arc, Mutex, MutexGuard};

/// Independent lifetime scope, even when another scope has the same file ID.
#[derive(Clone)]
pub struct DocumentRetention(
    Arc<DocumentAccount>,
    crate::model::json::lifetime::Shared<account::AccountIdentity>,
);
struct DocumentAccount {
    authority: RetentionAuthority,
    state: Mutex<DocumentState>,
}
struct DocumentState {
    roles: RetainedRoles,
    phases: phase_slots::PhaseRoles,
    identity: crate::model::json::lifetime::Shared<account::AccountIdentity>,
    pending: usize,
    pending_policy: usize,
}
impl DocumentRetention {
    /// Mint the only producer capabilities alongside their account. Capabilities
    /// cannot be synthesized by an ID lookup or cloned into another producer.
    pub fn new(authority: &RetentionAuthority) -> Result<(Self, DocumentRoles), RetentionError> {
        let fixed = Self::allocation_bytes()
            .checked_add(account::AccountIdentity::allocation_bytes())
            .and_then(|n| n.checked_add(RetentionAuthority::allocation_bytes()))
            .ok_or(RetentionError::Arithmetic)?;
        if fixed > crate::history::MAX_RESIDENT_BYTES {
            return Err(RetentionError::Limit {
                requested: fixed,
                available: crate::history::MAX_RESIDENT_BYTES,
            });
        }
        let identity = account::AccountIdentity::new(authority);
        let body = Arc::new(DocumentAccount {
            authority: authority.clone(),
            state: Mutex::new(DocumentState {
                roles: RetainedRoles::new(),
                phases: phase_slots::PhaseRoles::new(),
                identity: identity.clone(),
                pending: 0,
                pending_policy: 0,
            }),
        });
        identity.bind(&body);
        let scope = Self(body, identity);
        let roles = DocumentRoles::new(&scope)?;
        Ok((scope, roles))
    }
    pub fn authority(&self) -> &RetentionAuthority {
        &self.0.authority
    }
    pub fn same_document(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub fn retained_and_pending_bytes(&self) -> Result<usize, RetentionError> {
        let state = self.state()?;
        graph::measure_document_usage(self, &*state)?.used(state.pending, state.pending_policy)
    }
    fn allocation_bytes() -> usize {
        crate::model::allocations::shared_inline_bytes::<DocumentAccount>()
    }
    fn state(&self) -> Result<MutexGuard<'_, DocumentState>, RetentionError> {
        self.0
            .state
            .lock()
            .map_err(|_| RetentionError::DocumentPoisoned)
    }
    pub(crate) fn reserve_phase_metadata(
        &self,
        plan: AllocationPlan,
    ) -> Result<PendingAllocation, RetentionError> {
        self.reserve_checked(plan, None)
    }
    fn reserve_for_phase(
        &self,
        plan: AllocationPlan,
        phase: &phase::PhaseIdentity,
    ) -> Result<PendingAllocation, RetentionError> {
        self.reserve_checked(plan, Some(phase))
    }
    fn reserve_checked(
        &self,
        plan: AllocationPlan,
        phase: Option<&phase::PhaseIdentity>,
    ) -> Result<PendingAllocation, RetentionError> {
        let live = phase.map(phase::PhaseIdentity::upgrade).transpose()?;
        let mut state = self.state()?;
        if live.as_ref().is_some_and(|phase| !phase.is_active()) {
            return Err(RetentionError::FrozenOwner);
        }
        // This is the only lock order: document, then authentic census. The
        // graph guard has cleared every mark before graph::measure returns.
        let usage = graph::measure_document_usage(self, &*state)?;
        check_limit(usage.semantic(state.pending_policy)?)?;
        let used = usage.physical(state.pending)?;
        let available = crate::history::MAX_RESIDENT_BYTES.saturating_sub(used);
        if plan.bytes() > available {
            return Err(RetentionError::Limit {
                requested: plan.bytes(),
                available,
            });
        }
        state.pending = state
            .pending
            .checked_add(plan.bytes())
            .ok_or(RetentionError::Arithmetic)?;
        drop(state);
        Ok(PendingAllocation::new(self, plan, phase.cloned()))
    }
    fn reserve_semantic(
        &self,
        bytes: usize,
        phase: &phase::PhaseIdentity,
    ) -> Result<PolicyLiability, RetentionError> {
        let live = phase.upgrade()?;
        let mut state = self.state()?;
        if !live.is_active() {
            return Err(RetentionError::FrozenOwner);
        }
        let usage = graph::measure_document_usage(self, &*state)?;
        check_limit(usage.physical(state.pending)?)?;
        let used = usage.semantic(state.pending_policy)?;
        let available = crate::history::MAX_RESIDENT_BYTES.saturating_sub(used);
        if bytes > available {
            return Err(RetentionError::Limit {
                requested: bytes,
                available,
            });
        }
        state.pending_policy = state
            .pending_policy
            .checked_add(bytes)
            .ok_or(RetentionError::Arithmetic)?;
        Ok(PolicyLiability::new(self, bytes))
    }
    fn refund(&self, bytes: usize) {
        // Poison is fail-closed: no future admission can consume this account.
        // Retaining credit is conservative; payload retirement already occurred.
        if let Ok(mut state) = self.state() {
            state.pending = state
                .pending
                .checked_sub(bytes)
                .expect("authentic document credit");
        }
    }
    fn replace(
        &self,
        kind: RetainedKind,
        owner: &mut Option<RetainedPayload>,
        next: SnapshotStaging<'_>,
        mut policy: Option<&mut PolicyLiability>,
    ) -> Result<(), RetentionError> {
        if kind == RetainedKind::PromptDispatch {
            return Err(RetentionError::WrongPayload);
        }
        next.check_scope(self)?;
        let projection = RetainedProjection::Snapshot(next.projection());
        {
            let mut state = self.state()?;
            let consumed = policy
                .as_ref()
                .map(|token| token.checked_bytes(self))
                .transpose()?
                .unwrap_or(0);
            let remaining = state
                .pending_policy
                .checked_sub(consumed)
                .ok_or(RetentionError::Arithmetic)?;
            check_limit(
                graph::measure_document_union(self, &*state, &projection)?
                    .used(state.pending, remaining)?,
            )?;
            // No fallible state change after validation. Slots are touched only
            // through the unique producer capability and retire synchronously.
            graph::adopt(self, &projection, &mut state.pending)?;
            if let Some(policy) = policy.as_mut() {
                policy.consume(&mut state);
            }
            state.roles.retained_mut(kind).replace(Some(projection));
        }
        // Neither payload retirement nor last topology Arc Drop runs in a lock.
        let old = owner.replace(RetainedPayload::Snapshot(next.into_owner()));
        drop(old);
        self.finish_retiring(kind);
        Ok(())
    }
    fn replace_with_dispatch(
        &self,
        live: &mut Option<RetainedPayload>,
        dispatch: &mut Option<RetainedPayload>,
        next: SnapshotStaging<'_>,
        request: crate::history::physical::DispatchStaging<'_>,
    ) -> Result<(), RetentionError> {
        next.check_scope(self)?;
        request.check_scope(self)?;
        let live_projection = next.projection();
        let dispatch_projection = request.projection();
        let roots = payload::DispatchRoots::new(&live_projection, &dispatch_projection);
        {
            let mut state = self.state()?;
            state.validate_incoming(self, &roots)?;
            graph::adopt(self, &roots, &mut state.pending)?;
            state
                .roles
                .retained_mut(RetainedKind::Live)
                .replace(Some(RetainedProjection::Snapshot(live_projection)));
            state
                .roles
                .retained_mut(RetainedKind::PromptDispatch)
                .replace(Some(RetainedProjection::Dispatch(dispatch_projection)));
        }
        drop(live.replace(RetainedPayload::Snapshot(next.into_owner())));
        drop(dispatch.replace(RetainedPayload::Dispatch(request.into_payload())));
        self.finish_retiring(RetainedKind::Live);
        self.finish_retiring(RetainedKind::PromptDispatch);
        Ok(())
    }
    fn alias(
        &self,
        kind: RetainedKind,
        owner: &mut Option<RetainedPayload>,
        source: &SnapshotOwner,
    ) -> Result<(), RetentionError> {
        if kind == RetainedKind::PromptDispatch {
            return Err(RetentionError::WrongPayload);
        }
        let projection = RetainedProjection::Snapshot(source.projection());
        // Validate authentic authority without changing any producer state.
        graph::measure_document(self, &projection)?;
        let replacement = source.clone_within_role();
        {
            let mut state = self.state()?;
            state.validate_incoming(self, &projection)?;
            let slot = state.roles.retained_mut(kind);
            slot.replace(Some(projection));
        }
        let old = owner.replace(RetainedPayload::Snapshot(replacement));
        drop(old);
        self.finish_retiring(kind);
        Ok(())
    }
    fn clear(&self, kind: RetainedKind, owner: &mut Option<RetainedPayload>) {
        {
            let Ok(mut state) = self.state() else {
                drop(owner.take());
                return;
            };
            state.roles.retained_mut(kind).replace(None);
        }
        drop(owner.take());
        self.finish_retiring(kind);
    }
    fn finish_retiring(&self, kind: RetainedKind) {
        let (detached, credit) = {
            let Ok(mut state) = self.state() else {
                return;
            };
            let Some(retiring) = state.roles.retained_slot(kind).retiring() else {
                return;
            };
            let kept = view::Kept::retiring(&state, kind);
            let (_, delta) = graph::measure_suffix(self, &kept, retiring)
                .expect("previously admitted physical topology");
            let pending = state
                .pending
                .checked_add(delta)
                .expect("admitted metadata transfer");
            // Only infallible closed moves follow successful measurement.
            let detached = state
                .roles
                .retained_mut(kind)
                .take_retiring()
                .expect("borrowed retiring root");
            state.pending = pending;
            (detached, PendingAllocation::metadata(self, delta))
        };
        drop(detached);
        drop(credit);
    }
    fn publish_phase(
        &self,
        kind: ConstructionKind,
        next: phase::PhaseIdentity,
    ) -> Result<(), RetentionError> {
        graph::measure_document(self, &next)?;
        let mut state = self.state()?;
        state.validate_incoming(self, &next)?;
        graph::adopt(self, &next, &mut state.pending)?;
        state.phases.replace(kind, Some(next));
        Ok(())
    }
    fn clear_phase(&self, kind: ConstructionKind) {
        {
            let Ok(mut state) = self.state() else {
                return;
            };
            state.phases.replace(kind, None);
        }
        self.finish_phase_retiring(kind);
    }
    fn finish_phase_retiring(&self, kind: ConstructionKind) {
        let (detached, credit) = {
            let Ok(mut state) = self.state() else {
                return;
            };
            let Some(retiring) = state.phases.retiring(kind) else {
                return;
            };
            let kept = view::Kept::phase(&state, kind);
            let (_, delta) = graph::measure_suffix(self, &kept, retiring)
                .expect("admitted physical phase topology");
            let pending = state
                .pending
                .checked_add(delta)
                .expect("phase metadata transfer");
            let detached = state
                .phases
                .take_retiring(kind)
                .expect("borrowed phase retirement");
            state.pending = pending;
            (detached, PendingAllocation::metadata(self, delta))
        };
        drop(detached);
        drop(credit);
    }
    fn source(
        &self,
        kind: ConstructionKind,
        next: Option<SnapshotProjection>,
    ) -> Result<(), RetentionError> {
        let (detached, credit) = {
            let mut state = self.state()?;
            if let Some(projection) = &next {
                assert!(
                    state.roles.construction_slot(kind).is_none(),
                    "source replacement clears synchronously"
                );
                state.validate_incoming(self, projection)?;
                *state.roles.construction_mut(kind) = next;
                return Ok(());
            }
            let Some(source) = state.roles.construction_slot(kind) else {
                return Ok(());
            };
            let kept = view::Kept::source(&state, kind);
            let (_, delta) = graph::measure_suffix(self, &kept, source)?;
            let pending = state
                .pending
                .checked_add(delta)
                .ok_or(RetentionError::Arithmetic)?;
            let detached = state
                .roles
                .construction_mut(kind)
                .take()
                .expect("borrowed source root");
            state.pending = pending;
            (detached, PendingAllocation::metadata(self, delta))
        };
        drop(detached);
        drop(credit);
        Ok(())
    }
}
fn check_limit(used: usize) -> Result<(), RetentionError> {
    if used > crate::history::MAX_RESIDENT_BYTES {
        return Err(RetentionError::Limit {
            requested: used,
            available: crate::history::MAX_RESIDENT_BYTES,
        });
    }
    Ok(())
}
impl DocumentState {
    fn validate_incoming<G: OwnerGraph + ?Sized>(
        &self,
        scope: &DocumentRetention,
        incoming: &G,
    ) -> Result<(), RetentionError> {
        check_limit(
            graph::measure_document_union(scope, self, incoming)?
                .used(self.pending, self.pending_policy)?,
        )
    }
}
impl sealed::Source for DocumentState {}
impl OwnerGraph for DocumentState {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.identity.count(count)?;
        self.roles.count(count)?;
        self.phases.count(count)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.identity.cleanup(permit);
        self.roles.cleanup(permit);
        self.phases.cleanup(permit);
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.identity.adopt(publication);
        self.roles.adopt(publication);
        self.phases.adopt(publication);
    }
}

#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
mod policy_tests;
