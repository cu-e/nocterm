//! Private count-and-rollback protocol. A source has stable borrowed topology;
//! cleanup is sealed, allocation-free, infallible and invokes no external code.
use super::{CensusPermit, RetentionAuthority, RetentionCell, RetentionError};

pub(crate) mod sealed {
    pub(crate) trait Source {}
}

/// Implementations are actual domain owners, not user/adaptor callback objects.
/// Adapter-owned data must first expose domain-owned retained topology; no
/// arbitrary LegacySizing/HistoryJsonRetention call can run inside this protocol.
pub(crate) trait OwnerGraph: sealed::Source {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError>;
    fn cleanup(&self, permit: &CensusPermit<'_>);
    /// Sealed actual owner traversal; scratch tokens absent from this graph remain pending.
    fn adopt(&self, publication: &mut super::document::Publication<'_>);
}

pub(crate) struct Count<'p, 'a> {
    permit: &'p CensusPermit<'a>,
    bytes: usize,
    policy: usize,
    adoptable: usize,
    document: Option<&'a super::document::DocumentRetention>,
}
impl Count<'_, '_> {
    pub(crate) fn record_policy(&mut self, bytes: Option<usize>) -> Result<(), RetentionError> {
        if self.document.is_some() {
            self.policy = self
                .policy
                .max(bytes.ok_or(RetentionError::UncertifiedPolicy)?);
        }
        Ok(())
    }
    pub(crate) fn record_adoptable(
        &mut self,
        allocation: &super::document::PendingAllocation,
    ) -> Result<(), RetentionError> {
        if self.pays_pending_header(allocation) {
            self.adoptable = self
                .adoptable
                .checked_add(allocation.bytes())
                .ok_or(RetentionError::Arithmetic)?;
        }
        Ok(())
    }

    pub(crate) fn pays_pending_header(
        &self,
        allocation: &super::document::PendingAllocation,
    ) -> bool {
        allocation.is_pending()
            && self
                .document
                .is_some_and(|scope| allocation.belongs_to(scope))
    }
    pub(crate) fn validate_pending(
        &self,
        allocation: &super::document::PendingAllocation,
    ) -> Result<(), RetentionError> {
        if let Some(scope) = self.document
            && allocation.is_pending()
            && !allocation.belongs_to(scope)
        {
            return Err(RetentionError::ForeignAuthority);
        }
        Ok(())
    }

    pub(crate) fn add_exclusive(&mut self, bytes: usize) -> Result<(), RetentionError> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or(RetentionError::Arithmetic)?;
        Ok(())
    }
    pub(crate) fn validate_context(
        &self,
        authority: &RetentionAuthority,
    ) -> Result<(), RetentionError> {
        if self.permit.authority().same(authority) {
            Ok(())
        } else {
            Err(RetentionError::ForeignAuthority)
        }
    }
    /// Census never adopts persistent affinity. Caller-created IDs must pass
    /// through named owner intake before any side-effectful admission attempt.
    pub(crate) fn enter_id(
        &mut self,
        cell: &std::sync::OnceLock<RetentionCell>,
        requested: usize,
    ) -> Result<bool, RetentionError> {
        self.enter(
            cell.get().ok_or(RetentionError::MissingAuthority)?,
            requested,
        )
    }
    /// All callers supply a cost derived by the actual private allocation owner.
    /// False means the source must skip its entire already visited child graph.
    pub(crate) fn enter(
        &mut self,
        cell: &RetentionCell,
        requested: usize,
    ) -> Result<bool, RetentionError> {
        if !cell.first(self.permit)? {
            return Ok(false);
        }
        // The node stays marked even on overflow, so guard cleanup covers it.
        self.bytes = self
            .bytes
            .checked_add(requested)
            .ok_or(RetentionError::Arithmetic)?;
        Ok(true)
    }
}

struct Rollback<'g, 'a, G: OwnerGraph + ?Sized> {
    graph: &'g G,
    permit: CensusPermit<'a>,
}
impl<G: OwnerGraph + ?Sized> Drop for Rollback<'_, '_, G> {
    fn drop(&mut self) {
        // Every node clears itself BEFORE walking children and skips false nodes.
        // Thus shared subtrees are walked once, including an interrupted prefix.
        // All graph owners remain borrowed/pinned until this guard is gone.
        self.graph.cleanup(&self.permit);
    }
}

/// Both count and cleanup are linear in the visited real graph, not expanded
/// root-to-child paths. There is no independent preclear pass or persistent epoch.
/// Initial all-false follows from the previous sealed guard's cleanup. An abort
/// ends the process; ordinary error/unwind never returns with stale local marks.
pub(crate) fn measure<G: OwnerGraph + ?Sized>(
    authority: &RetentionAuthority,
    graph: &G,
) -> Result<usize, RetentionError> {
    measure_context(authority, graph, None)
}
pub(crate) fn measure_document<G: OwnerGraph + ?Sized>(
    scope: &super::document::DocumentRetention,
    graph: &G,
) -> Result<usize, RetentionError> {
    measure_context(scope.authority(), graph, Some(scope))
}
fn measure_context<G: OwnerGraph + ?Sized>(
    authority: &RetentionAuthority,
    graph: &G,
    document: Option<&super::document::DocumentRetention>,
) -> Result<usize, RetentionError> {
    let mut permit = authority.permit();
    permit.begin()?;
    let rollback = Rollback { graph, permit };
    let mut count = Count {
        permit: &rollback.permit,
        bytes: RetentionAuthority::allocation_bytes(),
        policy: 0,
        adoptable: 0,
        document,
    };
    graph.count(&mut count)?;
    let bytes = count.bytes;
    drop(rollback);
    Ok(bytes)
}

/// Actual union and immutable maximum root policy are independent observations.
/// Pending bytes already present in this graph are removed virtually before a
/// commit, without changing any authentic token on a failed admission.
pub(crate) struct CensusUsage {
    actual: usize,
    policy: usize,
    adoptable: usize,
}
impl CensusUsage {
    pub(crate) fn physical(&self, pending: usize) -> Result<usize, RetentionError> {
        self.actual
            .checked_add(
                pending
                    .checked_sub(self.adoptable)
                    .ok_or(RetentionError::Arithmetic)?,
            )
            .ok_or(RetentionError::Arithmetic)
    }
    pub(crate) fn semantic(&self, source_policy: usize) -> Result<usize, RetentionError> {
        self.policy
            .checked_add(source_policy)
            .ok_or(RetentionError::Arithmetic)
    }
    pub(crate) fn used(
        &self,
        pending: usize,
        source_policy: usize,
    ) -> Result<usize, RetentionError> {
        Ok(self.physical(pending)?.max(self.semantic(source_policy)?))
    }
}
pub(crate) fn measure_document_usage<G: OwnerGraph + ?Sized>(
    scope: &super::document::DocumentRetention,
    graph: &G,
) -> Result<CensusUsage, RetentionError> {
    let mut permit = scope.authority().permit();
    permit.begin()?;
    let rollback = Rollback { graph, permit };
    let mut count = Count {
        permit: &rollback.permit,
        bytes: RetentionAuthority::allocation_bytes(),
        policy: 0,
        adoptable: 0,
        document: Some(scope),
    };
    graph.count(&mut count)?;
    let usage = CensusUsage {
        actual: count.bytes,
        policy: count.policy,
        adoptable: count.adoptable,
    };
    drop(rollback);
    Ok(usage)
}

/// Count a genuine detached suffix after all kept roots with one mark set.
/// The suffix increment includes its unique topology/header backing even after
/// its payload retires. Neither operation invokes externally supplied code.
pub(super) fn measure_suffix<K: OwnerGraph + ?Sized, S: OwnerGraph + ?Sized>(
    scope: &super::document::DocumentRetention,
    kept: &K,
    suffix: &S,
) -> Result<(usize, usize), RetentionError> {
    struct Pair<'a, K: ?Sized, S: ?Sized>(&'a K, &'a S);
    impl<K: OwnerGraph + ?Sized, S: OwnerGraph + ?Sized> sealed::Source for Pair<'_, K, S> {}
    impl<K: OwnerGraph + ?Sized, S: OwnerGraph + ?Sized> OwnerGraph for Pair<'_, K, S> {
        fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
            self.0.count(count)?;
            self.1.count(count)
        }
        fn cleanup(&self, permit: &CensusPermit<'_>) {
            self.0.cleanup(permit);
            self.1.cleanup(permit);
        }
        fn adopt(&self, publication: &mut super::document::Publication<'_>) {
            self.0.adopt(publication);
            self.1.adopt(publication);
        }
    }
    let mut permit = scope.authority().permit();
    permit.begin()?;
    let pair = Pair(kept, suffix);
    let rollback = Rollback {
        graph: &pair,
        permit,
    };
    let mut count = Count {
        permit: &rollback.permit,
        bytes: RetentionAuthority::allocation_bytes(),
        policy: 0,
        adoptable: 0,
        document: Some(scope),
    };
    kept.count(&mut count)?;
    let prefix = count.bytes;
    suffix.count(&mut count)?;
    let increment = count.bytes - prefix;
    drop(rollback);
    Ok((prefix, increment))
}

/// A second sealed, mark-before-children pass adopts exactly actual reachable
/// tokens. Scratch absent from the physical topology stays pending.
pub(super) fn adopt<G: OwnerGraph + ?Sized>(
    scope: &super::document::DocumentRetention,
    graph: &G,
    pending: &mut usize,
) -> Result<(), RetentionError> {
    struct Adoption<'g, 'a, G: OwnerGraph + ?Sized> {
        graph: &'g G,
        publication: super::document::Publication<'a>,
    }
    impl<G: OwnerGraph + ?Sized> Drop for Adoption<'_, '_, G> {
        fn drop(&mut self) {
            self.graph.cleanup(self.publication.permit());
        }
    }
    let publication = super::document::Publication::new(scope, pending)?;
    let mut guard = Adoption { graph, publication };
    graph.adopt(&mut guard.publication);
    drop(guard);
    Ok(())
}

/// Borrowed closed-owner union for atomic admission before authentic adoption.
struct Union<'a, A: ?Sized, B: ?Sized>(&'a A, &'a B);
impl<A: OwnerGraph + ?Sized, B: OwnerGraph + ?Sized> sealed::Source for Union<'_, A, B> {}
impl<A: OwnerGraph + ?Sized, B: OwnerGraph + ?Sized> OwnerGraph for Union<'_, A, B> {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.0.count(count)?;
        self.1.count(count)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.0.cleanup(permit);
        self.1.cleanup(permit);
    }
    fn adopt(&self, publication: &mut super::document::Publication<'_>) {
        self.0.adopt(publication);
        self.1.adopt(publication);
    }
}
pub(crate) fn measure_document_union<A: OwnerGraph + ?Sized, B: OwnerGraph + ?Sized>(
    scope: &super::document::DocumentRetention,
    current: &A,
    incoming: &B,
) -> Result<CensusUsage, RetentionError> {
    measure_document_usage(scope, &Union(current, incoming))
}
