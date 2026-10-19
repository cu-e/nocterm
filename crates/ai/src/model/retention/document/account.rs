//! Authentic account weak headers remain visible across genuine document forks.
//! A receipt pins only the real Arc block, never its document payload/state.
use super::{DocumentAccount, DocumentRetention, RetentionError};
use crate::model::{
    json::lifetime::Shared,
    retention::{
        CensusPermit, RetentionAuthority, RetentionCell,
        graph::{Count, OwnerGraph, sealed},
    },
};
use std::sync::{Arc, OnceLock, Weak};
pub(super) struct AccountIdentity {
    // Drops before any remaining credit owned by a future account allocator.
    header: OnceLock<Weak<dyn std::any::Any + Send + Sync>>,
    cell: RetentionCell,
}
impl AccountIdentity {
    pub(super) fn new(authority: &RetentionAuthority) -> Shared<Self> {
        Shared::new(Self {
            header: OnceLock::new(),
            cell: RetentionCell::new(authority),
        })
    }
    pub(super) fn bind(&self, account: &Arc<DocumentAccount>) {
        // Erase only the genuine weak header. Operational upgrade performs a
        // checked downcast outside census; no payload graph auto-trait cycle
        // leaks through this otherwise metadata-only identity leaf.
        let erased: Arc<dyn std::any::Any + Send + Sync> = account.clone();
        assert!(
            self.header.set(Arc::downgrade(&erased)).is_ok(),
            "fresh account binding"
        );
    }
    pub(super) fn same(&self, other: &Self) -> bool {
        self.header
            .get()
            .expect("bound account")
            .ptr_eq(other.header.get().expect("bound account"))
    }
    pub(super) fn upgrade(identity: &Shared<Self>) -> Option<DocumentRetention> {
        identity
            .header
            .get()?
            .upgrade()?
            .downcast::<DocumentAccount>()
            .ok()
            .map(|account| DocumentRetention(account, identity.clone()))
    }
    pub(super) fn allocation_bytes() -> usize {
        crate::model::allocations::shared_inline_bytes::<Self>()
    }
}
impl sealed::Source for Shared<AccountIdentity> {}
impl OwnerGraph for Shared<AccountIdentity> {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        let bytes = DocumentRetention::allocation_bytes()
            .checked_add(AccountIdentity::allocation_bytes())
            .ok_or(RetentionError::Arithmetic)?;
        count.enter(&self.cell, bytes)?;
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.cell.clear_owned(permit);
    }
    fn adopt(&self, publication: &mut super::Publication<'_>) {
        publication.enter(&self.cell);
    }
}
