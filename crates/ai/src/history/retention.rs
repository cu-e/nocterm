//! Authentic legacy provenance storage; format ordering remains in the adapter.
//! The generic facts are Copy and own no destructors. Census traverses only
//! domain identity children, never an adapter function or arbitrary sizing sink.
use crate::model::json::{
    deferred::{DecodeLedger, JsonDecodeError, ledger::Reservation},
    lifetime::Shared,
};
use crate::model::{
    extensions::ExtensionsIdentity,
    ids::IdentifierIdentity,
    retention::{
        CensusPermit, RetentionAuthority, RetentionCell, RetentionError,
        graph::{Count, OwnerGraph, sealed},
    },
    shared_display::DisplayIdentity,
    tools::ToolRecord,
};
use std::sync::{Arc, OnceLock, Weak};

pub struct LegacyRowIdentity {
    id: IdentifierIdentity,
    display: DisplayIdentity,
    extensions: Option<ExtensionsIdentity>,
}
impl LegacyRowIdentity {
    pub fn capture(row: &ToolRecord) -> Option<Self> {
        Some(Self {
            id: row.id.retention_identity(),
            display: row.client_display.as_ref()?.identity(),
            extensions: row.extensions.as_ref().map(|value| value.identity()),
        })
    }
    pub fn matches(&self, row: &ToolRecord) -> bool {
        self.id.matches_tool(&row.id)
    }
    pub fn matches_managed(&self, row: &crate::history::physical::ToolSlot) -> bool {
        self.id.matches_tool(&row.id())
    }
    pub fn matches_managed_display(&self, row: &crate::history::physical::ToolSlot) -> bool {
        row.client_display()
            .as_ref()
            .is_some_and(|value| self.display.matches(value))
    }
    pub fn matches_managed_extensions(&self, row: &crate::history::physical::ToolSlot) -> bool {
        match (&self.extensions, row.extensions()) {
            (None, None) => true,
            (Some(identity), Some(current)) => identity.matches(&current),
            _ => false,
        }
    }
    pub fn matches_display(&self, row: &ToolRecord) -> bool {
        row.client_display
            .as_ref()
            .is_some_and(|value| self.display.matches(value))
    }
    pub fn matches_extensions(&self, row: &ToolRecord) -> bool {
        match (&self.extensions, &row.extensions) {
            (None, None) => true,
            (Some(identity), Some(current)) => identity.matches(current),
            _ => false,
        }
    }
    pub fn retained_heap_bytes(&self) -> Option<usize> {
        self.id
            .retained_heap_bytes()?
            .checked_add(self.display.inline_bytes())?
            .checked_add(
                self.extensions
                    .as_ref()
                    .map_or(Some(0), ExtensionsIdentity::retained_heap_bytes)?,
            )
    }
    pub fn visit_json_allocations<'a>(
        &'a self,
        visit: &mut dyn FnMut(
            crate::model::json::RetainedAllocation<'a>,
        ) -> Result<(), super::RetentionError>,
    ) -> Result<(), super::RetentionError> {
        if let Some(identity) = &self.extensions {
            for component in identity.retained_allocations().into_iter().flatten() {
                visit(component)?;
            }
        }
        Ok(())
    }
}
impl sealed::Source for LegacyRowIdentity {}
impl OwnerGraph for LegacyRowIdentity {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.id.count(count)?;
        self.display.count(count)?;
        if let Some(extensions) = &self.extensions {
            extensions.count(count)?;
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.id.cleanup(permit);
        self.display.cleanup(permit);
        if let Some(extensions) = &self.extensions {
            extensions.cleanup(permit);
        }
    }

    fn adopt(&self, publication: &mut crate::model::retention::document::Publication<'_>) {
        self.id.adopt(publication);
        self.display.adopt(publication);
        if let Some(extensions) = &self.extensions {
            extensions.adopt(publication);
        }
    }
}
/// This is the actual row storage, not a second index or a list of estimates.
pub struct LegacyRow<F: Copy> {
    identity: LegacyRowIdentity,
    facts: F,
}
impl<F: Copy> LegacyRow<F> {
    pub fn new(identity: LegacyRowIdentity, facts: F) -> Self {
        Self { identity, facts }
    }
    pub fn identity(&self) -> &LegacyRowIdentity {
        &self.identity
    }
    pub fn facts(&self) -> &F {
        &self.facts
    }
}
struct RowsLease {
    header: OnceLock<Weak<dyn OwnerGraph + Send + Sync>>,
    retention: RetentionCell,
    // Weak physically frees the Rows header before its reservation retires.
    allocation: Option<Reservation>,
}
struct Rows<F: Copy> {
    values: Vec<LegacyRow<F>>,
    lease: Shared<RowsLease>,
}
pub struct LegacyRows<F: Copy> {
    rows: Arc<Rows<F>>,
    lease: Shared<RowsLease>,
}
impl<F: Copy> Clone for LegacyRows<F> {
    fn clone(&self) -> Self {
        Self {
            rows: self.rows.clone(),
            lease: self.lease.clone(),
        }
    }
}
impl<F: Copy + Send + Sync + 'static> LegacyRows<F> {
    /// Compatibility intake moves actual existing reserve; controlled native
    /// construction reserves this header and vector before creating them.
    pub fn from_untracked(authority: &RetentionAuthority, values: Vec<LegacyRow<F>>) -> Self {
        Self::build(authority, values, None)
    }
    pub fn admitted(ledger: &DecodeLedger, capacity: usize) -> Result<Self, JsonDecodeError> {
        let bytes = capacity
            .checked_mul(std::mem::size_of::<LegacyRow<F>>())
            .and_then(|bytes| bytes.checked_add(Self::inline_bytes()))
            .ok_or(JsonDecodeError::Arithmetic)?;
        let allocation = ledger.reserve(bytes)?;
        let mut values = Vec::new();
        values.try_reserve_exact(capacity)?;
        if values.capacity() != capacity {
            return Err(JsonDecodeError::CapacityProfile);
        }
        Ok(Self::build(ledger.authority(), values, Some(allocation)))
    }
    fn build(
        authority: &RetentionAuthority,
        values: Vec<LegacyRow<F>>,
        allocation: Option<Reservation>,
    ) -> Self {
        let lease = Shared::new(RowsLease {
            header: OnceLock::new(),
            retention: RetentionCell::new(authority),
            allocation,
        });
        let rows = Arc::new(Rows {
            values,
            lease: lease.clone(),
        });
        Self { rows, lease }
    }
    pub fn push(&mut self, row: LegacyRow<F>) -> Result<(), RetentionError> {
        Arc::get_mut(&mut self.rows)
            .ok_or(RetentionError::FrozenOwner)?
            .values
            .push(row);
        Ok(())
    }
    pub fn push_admitted(&mut self, row: LegacyRow<F>) -> Result<(), JsonDecodeError> {
        if self.lease.allocation.is_none() {
            return Err(JsonDecodeError::ConstructionOrder);
        }
        let rows = Arc::get_mut(&mut self.rows).ok_or(JsonDecodeError::ConstructionOrder)?;
        if rows.values.len() == rows.values.capacity() {
            return Err(JsonDecodeError::ConstructionOrder);
        }
        rows.values.push(row);
        Ok(())
    }
    pub fn iter(&self) -> impl Iterator<Item = &LegacyRow<F>> {
        self.rows.values.iter()
    }
    pub fn capacity(&self) -> usize {
        self.rows.values.capacity()
    }
    pub fn inline_bytes() -> usize {
        crate::model::allocations::shared_inline_bytes::<Rows<F>>()
            + crate::model::allocations::shared_inline_bytes::<RowsLease>()
    }
    pub fn source(&self) -> LegacyRetentionSource {
        let rows: Arc<dyn OwnerGraph + Send + Sync> = self.rows.clone();
        self.lease.header.get_or_init(|| Arc::downgrade(&rows));
        LegacyRetentionSource {
            rows,
            _lease: self.lease.clone(),
        }
    }
}
impl<F: Copy + Send + Sync + 'static> sealed::Source for Rows<F> {}
impl<F: Copy + Send + Sync + 'static> OwnerGraph for Rows<F> {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        let bytes = self
            .values
            .capacity()
            .checked_mul(std::mem::size_of::<LegacyRow<F>>())
            .and_then(|capacity| capacity.checked_add(LegacyRows::<F>::inline_bytes()))
            .ok_or(RetentionError::Arithmetic)?;
        if count.enter(&self.lease.retention, bytes)? {
            if let Some(allocation) = &self.lease.allocation {
                allocation.validate_pending(count)?;
            }
            for row in &self.values {
                row.identity.count(count)?;
            }
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        if self.lease.retention.clear_owned(permit) {
            if let Some(allocation) = &self.lease.allocation {
                allocation.cleanup_pending(permit);
            }
            for row in &self.values {
                row.identity.cleanup(permit);
            }
        }
    }

    fn adopt(&self, publication: &mut crate::model::retention::document::Publication<'_>) {
        if publication.enter(&self.lease.retention) {
            if let Some(allocation) = &self.lease.allocation {
                allocation.adopt_under(publication);
            }
            for row in &self.values {
                row.identity.adopt(publication);
            }
        }
    }
}
/// Only domain-owned concrete storage can create this opaque source. Its sealed
/// vtable performs no operation on adapter facts and calls no adapter callback.
#[derive(Clone)]
pub struct LegacyRetentionSource {
    rows: Arc<dyn OwnerGraph + Send + Sync>,
    _lease: Shared<RowsLease>,
}
impl sealed::Source for LegacyRetentionSource {}
impl OwnerGraph for LegacyRetentionSource {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.rows.count(count)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.rows.cleanup(permit);
    }

    fn adopt(&self, publication: &mut crate::model::retention::document::Publication<'_>) {
        self.rows.adopt(publication);
    }
}

mod legacy;
pub use legacy::{
    LegacyFormat, LegacyIdentity, legacy_value, retain_legacy, retain_legacy_admitted,
};
