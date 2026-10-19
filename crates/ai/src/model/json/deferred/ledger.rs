//! Requested allocations owned by one decode operation, including released peaks.
use super::JsonDecodeError;
mod admission;
mod scoped;
use crate::model::json::lifetime::Shared;
use crate::model::retention::{
    CensusPermit, RetentionError,
    document::{ConstructionRole, DocumentRetention, PendingAllocation, Publication},
    graph::Count,
};
use crate::model::retention::{RetentionAuthority, RetentionCell};
pub use admission::NativeAllocationError;
use scoped::ScopedLedger;
use std::sync::{
    Mutex, MutexGuard,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Clone, Copy, Debug)]
struct Usage {
    live: usize,
    peak: usize,
    limit: usize,
    closed: bool,
}
/// One shared pure construction budget retained by actual allocation owners.
/// Callers must reserve already retained siblings.
/// It does not meter the stock deserializer's private scratch automatically.
#[derive(Debug)]
pub struct DecodeLedger(Shared<LedgerOwner>);
#[derive(Debug)]
struct LedgerOwner {
    usage: Mutex<Usage>,
    retention: RetentionCell,
    scope: Option<ScopedLedger>,
}
impl DecodeLedger {
    pub(crate) fn share(&self) -> Self {
        Self(self.0.clone())
    }
    pub fn new(authority: &RetentionAuthority, limit: usize) -> Result<Self, JsonDecodeError> {
        Self::build(authority, limit, None)
    }
    pub(crate) fn for_operation(
        role: &ConstructionRole,
        limit: usize,
    ) -> Result<Self, JsonDecodeError> {
        let fixed = crate::model::allocations::shared_inline_bytes::<LedgerOwner>();
        let scope = ScopedLedger::new(role, fixed)?;
        Self::build(role.scope().authority(), limit, Some(scope))
    }
    pub(crate) fn for_source(
        role: &ConstructionRole,
        limit: usize,
    ) -> Result<Self, JsonDecodeError> {
        let fixed = crate::model::allocations::shared_inline_bytes::<LedgerOwner>();
        let scope = ScopedLedger::for_source(role, fixed)?;
        Self::build(role.scope().authority(), limit, Some(scope))
    }
    fn build(
        authority: &RetentionAuthority,
        limit: usize,
        scope: Option<ScopedLedger>,
    ) -> Result<Self, JsonDecodeError> {
        let fixed = crate::model::allocations::shared_inline_bytes::<LedgerOwner>();
        if fixed > limit {
            return Err(JsonDecodeError::Limit {
                requested: fixed,
                available: limit,
            });
        }
        Ok(Self(Shared::new(LedgerOwner {
            usage: Mutex::new(Usage {
                live: fixed,
                peak: fixed,
                limit,
                closed: false,
            }),
            retention: RetentionCell::new(authority),
            scope,
        })))
    }
    pub fn authority(&self) -> &RetentionAuthority {
        self.0.retention.authority()
    }
    /// A borrowed identity of this actual ledger block, never its unrelated live usage.
    pub(crate) fn allocation_component(&self) -> super::super::RetainedAllocation<'_> {
        super::super::RetainedAllocation::ledger(self)
    }
    pub(in crate::model::json) fn same_allocation(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
    pub(in crate::model::json) fn allocation_bytes(&self) -> usize {
        crate::model::allocations::shared_inline_bytes::<LedgerOwner>()
    }
    fn usage(&self) -> MutexGuard<'_, Usage> {
        self.0
            .usage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    pub(super) fn available(&self) -> usize {
        let state = self.usage();
        state.limit - state.live
    }
    pub fn live_requested_bytes(&self) -> usize {
        self.usage().live
    }
    pub fn peak_requested_bytes(&self) -> usize {
        self.usage().peak
    }
    pub fn belongs_to_document(&self, document: &DocumentRetention) -> bool {
        self.0
            .scope
            .as_ref()
            .is_some_and(|scope| scope.belongs_to(document))
    }
    pub(crate) fn reserve_source_policy(
        &self,
        bytes: usize,
    ) -> Result<crate::model::retention::document::PolicyLiability, JsonDecodeError> {
        self.0
            .scope
            .as_ref()
            .ok_or(JsonDecodeError::Retention(RetentionError::MissingAuthority))?
            .reserve_policy(bytes)
    }
    pub(crate) fn reserve(&self, bytes: usize) -> Result<Reservation, JsonDecodeError> {
        self.reserve_requested(bytes).map_err(Into::into)
    }
    pub(crate) fn reserve_requested(
        &self,
        bytes: usize,
    ) -> Result<Reservation, NativeAllocationError> {
        // Document reservation runs before the diagnostic Usage lock. Failure
        // refunds its pending receipt only after that lock has been released.
        let pending = self
            .0
            .scope
            .as_ref()
            .map(|scope| scope.reserve(bytes))
            .transpose()?;
        let mut state = self.usage();
        if state.closed {
            drop(state);
            drop(pending);
            return Err(NativeAllocationError::Retention(
                RetentionError::FrozenOwner,
            ));
        }
        let next = state
            .live
            .checked_add(bytes)
            .ok_or(NativeAllocationError::Arithmetic);
        let next = match next {
            Ok(next) if next <= state.limit => next,
            result => {
                let error = result.err().unwrap_or(NativeAllocationError::Limit {
                    requested: bytes,
                    available: state.limit - state.live,
                });
                drop(state);
                drop(pending);
                return Err(error);
            }
        };
        state.live = next;
        state.peak = state.peak.max(next);
        drop(state);
        Ok(Reservation {
            ledger: self.share(),
            bytes: AtomicUsize::new(bytes),
            pending,
        })
    }
    pub(crate) fn close_image_operation(&self) {
        self.usage().closed = true;
    }
}
#[derive(Debug)]
pub(crate) struct Reservation {
    ledger: DecodeLedger,
    bytes: AtomicUsize,
    pending: Option<PendingAllocation>,
}
impl Reservation {
    /// Immutable payload affinity uses the actual paid account identity even
    /// after the producer's allocation phase has physically retired.
    pub(crate) fn belongs_to_document(&self, scope: &DocumentRetention) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| pending.belongs_to(scope))
    }
    pub(crate) fn original_ledger(&self) -> &DecodeLedger {
        &self.ledger
    }
    pub(crate) fn belongs_to(&self, ledger: &DecodeLedger) -> bool {
        self.ledger.0.ptr_eq(&ledger.0)
    }
    pub(crate) fn bytes(&self) -> usize {
        self.bytes.load(Ordering::Acquire)
    }
    pub(crate) fn merge(&mut self, mut other: Self) -> Result<(), JsonDecodeError> {
        self.merge_from(&mut other)
    }
    /// Failure leaves both owners and their live reservations intact.
    pub(crate) fn merge_from(&mut self, other: &mut Self) -> Result<(), JsonDecodeError> {
        if !self.ledger.0.ptr_eq(&other.ledger.0) {
            return Err(JsonDecodeError::ForeignLedger);
        }
        let bytes = self
            .bytes()
            .checked_add(other.bytes())
            .ok_or(JsonDecodeError::Arithmetic)?;
        match (&mut self.pending, &mut other.pending) {
            (Some(left), Some(right)) => left.merge_from(right).map_err(scoped::error)?,
            (None, None) => {}
            _ => return Err(JsonDecodeError::ForeignLedger),
        }
        *self.bytes.get_mut() = bytes;
        *other.bytes.get_mut() = 0;
        Ok(())
    }
    pub(crate) fn split(&mut self, bytes: usize) -> Result<Self, JsonDecodeError> {
        let remaining = self
            .bytes()
            .checked_sub(bytes)
            .ok_or(JsonDecodeError::Arithmetic)?;
        let pending = self
            .pending
            .as_mut()
            .map(|pending| pending.split(bytes))
            .transpose()
            .map_err(scoped::error)?;
        *self.bytes.get_mut() = remaining;
        Ok(Self {
            ledger: self.ledger.share(),
            bytes: AtomicUsize::new(bytes),
            pending,
        })
    }
    pub(crate) fn grow_requested(&mut self, bytes: usize) -> Result<(), NativeAllocationError> {
        let mut additional = self.ledger.reserve_requested(bytes)?;
        self.merge_from(&mut additional)
            .expect("fresh original ledger reservation bounded by its live count");
        Ok(())
    }
    pub(crate) fn grow(&mut self, bytes: usize) -> Result<(), JsonDecodeError> {
        let mut additional = self.ledger.reserve(bytes)?;
        self.merge_from(&mut additional)
    }
    pub(crate) fn validate_pending(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        if let Some(pending) = &self.pending {
            pending.validate_for_count(count)?;
        }
        crate::model::retention::graph::OwnerGraph::count(&self.ledger, count)
    }
    pub(crate) fn cleanup_pending(&self, permit: &CensusPermit<'_>) {
        if let Some(pending) = &self.pending {
            pending.cleanup_for_count(permit);
        }
        crate::model::retention::graph::OwnerGraph::cleanup(&self.ledger, permit);
    }
    pub(crate) fn adopt_under(&self, publication: &mut Publication<'_>) {
        if let Some(pending) = &self.pending {
            pending.adopt_under(publication);
        }
        crate::model::retention::graph::OwnerGraph::adopt(&self.ledger, publication);
    }
    /// Only the actual payload owner calls this, after its backing has retired.
    /// Weak metadata retains the genuine zero-byte receipt for census/adoption.
    pub(crate) fn retire_after_physical(&self) {
        let bytes = self.bytes.swap(0, Ordering::AcqRel);
        if bytes != 0 {
            let mut state = self.ledger.usage();
            state.live = state
                .live
                .checked_sub(bytes)
                .expect("authentic reservation credit underflow");
        }
        if let Some(pending) = &self.pending {
            pending.retire_after_physical();
        }
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.retire_after_physical();
    }
}

#[cfg(test)]
mod tests;

impl crate::model::retention::graph::sealed::Source for DecodeLedger {}
impl crate::model::retention::graph::OwnerGraph for DecodeLedger {
    fn count(
        &self,
        count: &mut crate::model::retention::graph::Count<'_, '_>,
    ) -> Result<(), crate::model::retention::RetentionError> {
        let header_bytes = if self
            .0
            .scope
            .as_ref()
            .is_some_and(|scope| scope.header_paid_here(count))
        {
            0
        } else {
            self.allocation_bytes()
        };
        if count.enter(&self.0.retention, header_bytes)?
            && let Some(scope) = &self.0.scope
        {
            scope.count(count)?;
        }
        Ok(())
    }
    fn cleanup(&self, permit: &crate::model::retention::CensusPermit<'_>) {
        if self.0.retention.clear_owned(permit)
            && let Some(scope) = &self.0.scope
        {
            scope.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        if publication.enter(&self.0.retention)
            && let Some(scope) = &self.0.scope
        {
            scope.adopt(publication);
        }
    }
}
