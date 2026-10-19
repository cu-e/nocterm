//! Explicit coordination of authentic retained owners, independent of decode ledgers.
//!
//! Allocation marks are private visitation state. They carry no caller-supplied
//! address, byte count or reservation. Graph admission owns validation/clear/count;
//! the authority never invokes an external source or sink while holding its lock.
pub(crate) mod graph;
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug, thiserror::Error)]
pub enum RetentionError {
    #[error("retained payload has no authentic semantic policy proof")]
    UncertifiedPolicy,
    #[error("retained capability carries another payload kind")]
    WrongPayload,
    #[error("document retention bookkeeping was poisoned")]
    DocumentPoisoned,
    #[error("retained owner is already shared and immutable")]
    FrozenOwner,
    #[error("untracked retained owner requires explicit context intake before census")]
    MissingAuthority,
    #[error("retention allocation arithmetic overflow")]
    Arithmetic,
    #[error("retention metadata requests {requested} bytes with {available} available")]
    Limit { requested: usize, available: usize },
    #[error("retained owner belongs to a different injected authority")]
    ForeignAuthority,
    #[error("retained graph census is already active on this permit")]
    ReentrantCensus,
}

#[derive(Debug, Default)]
struct Phase {
    active: bool,
}

/// One host-supplied coordination context; separate ledgers retain their budgets.
#[derive(Clone, Debug)]
pub struct RetentionAuthority(Arc<Mutex<Phase>>);
impl RetentionAuthority {
    /// Check the actual fixed allocation before creating the context. This is not
    /// a reservation for later payloads or a proof of their complete peak.
    pub fn new(available: usize) -> Result<Self, RetentionError> {
        let requested = Self::allocation_bytes();
        if requested > available {
            return Err(RetentionError::Limit {
                requested,
                available,
            });
        }
        Ok(Self(Arc::new(Mutex::new(Phase::default()))))
    }

    pub fn allocation_bytes() -> usize {
        super::allocations::shared_inline_bytes::<Mutex<Phase>>()
    }

    pub(crate) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Acquired outside traversal and other budget/cleanup/native locks. The
    /// permit is never supplied to arbitrary callbacks or acquisition closures.
    pub(crate) fn permit(&self) -> CensusPermit<'_> {
        CensusPermit {
            authority: self,
            phase: self
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        }
    }
}

pub(crate) struct CensusPermit<'a> {
    authority: &'a RetentionAuthority,
    phase: MutexGuard<'a, Phase>,
}
impl CensusPermit<'_> {
    pub(crate) fn authority(&self) -> &RetentionAuthority {
        self.authority
    }

    /// Nested internal begin is rejected on an already held permit. This does
    /// not claim to detect an arbitrary same-thread second mutex acquisition.
    pub(crate) fn begin(&mut self) -> Result<(), RetentionError> {
        if self.phase.active {
            return Err(RetentionError::ReentrantCensus);
        }
        self.phase.active = true;
        Ok(())
    }
}
impl Drop for CensusPermit<'_> {
    fn drop(&mut self) {
        self.phase.active = false;
    }
}

/// Inline metadata is included in the containing owner's real allocation plan.
/// A clone of this cell would mint a second mark for one allocation and is forbidden.
#[derive(Debug)]
pub(crate) struct RetentionCell {
    authority: RetentionAuthority,
    marked: AtomicBool,
}
impl RetentionCell {
    pub(crate) fn new(authority: &RetentionAuthority) -> Self {
        Self {
            authority: authority.clone(),
            marked: AtomicBool::new(false),
        }
    }
    pub(crate) fn authority(&self) -> &RetentionAuthority {
        &self.authority
    }
    pub(crate) fn belongs_to(&self, authority: &RetentionAuthority) -> bool {
        self.authority.same(authority)
    }
    pub(crate) fn validate(&self, permit: &CensusPermit<'_>) -> Result<(), RetentionError> {
        if self.authority.same(permit.authority) {
            Ok(())
        } else {
            Err(RetentionError::ForeignAuthority)
        }
    }
    /// Infallible rollback: foreign nodes were never entered and are never
    /// inspected/mutated through this authority. False skips a shared subtree.
    pub(crate) fn clear_owned(&self, permit: &CensusPermit<'_>) -> bool {
        self.authority.same(permit.authority) && self.marked.swap(false, Ordering::Relaxed)
    }
    pub(crate) fn first(&self, permit: &CensusPermit<'_>) -> Result<bool, RetentionError> {
        self.validate(permit)?;
        Ok(!self.marked.swap(true, Ordering::Relaxed))
    }
}

pub mod document;
