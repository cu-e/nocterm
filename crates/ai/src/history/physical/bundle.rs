//! The actual typed allocation/lifetime mechanism shared by concrete parts.
//! Implementations are private domain storage types, never caller callbacks.
use super::slots::{NativeCredit, PhysicalError, SlabBuilder};
use crate::model::{
    json::lifetime::Shared,
    retention::{
        CensusPermit, RetentionCell, RetentionError,
        document::{ConstructionRole, PendingAllocation, Publication},
        graph::{Count, OwnerGraph, sealed},
    },
};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

pub(super) trait Body {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError>;
    fn cleanup(&self, permit: &CensusPermit<'_>);
    fn adopt(&self, publication: &mut Publication<'_>);
    fn retire(&self);
}
struct Allocation<T: Body> {
    body: T,
    cell: RetentionCell,
    retired: AtomicBool,
    original: Shared<NativeCredit>,
    credit: PendingAllocation,
}
struct Life<T: Body> {
    slab: Shared<Allocation<T>>,
}
impl<T: Body> Drop for Life<T> {
    fn drop(&mut self) {
        self.slab.body.retire();
        self.slab.retired.store(true, Ordering::Release);
    }
}
pub(super) struct BundleOwner<T: Body> {
    life: Shared<Life<T>>,
}
pub(super) struct BundleProjection<T: Body>(Shared<Allocation<T>>);
impl<T: Body> Clone for BundleProjection<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T: Body> BundleOwner<T> {
    pub(super) fn build(
        body: T,
        slab: SlabBuilder<'_>,
        original: &Shared<NativeCredit>,
    ) -> Result<Self, PhysicalError> {
        struct Unpublished<'a, T> {
            body: T,
            slab: SlabBuilder<'a>,
        }
        let mut staging = Unpublished { body, slab };
        staging.slab.shared::<Allocation<T>>()?;
        staging.slab.shared::<Life<T>>()?;
        let cell = RetentionCell::new(staging.slab.role().scope().authority());
        let allocation = Shared::new(Allocation {
            body: staging.body,
            cell,
            retired: AtomicBool::new(false),
            original: original.clone(),
            credit: staging.slab.finish(),
        });
        Ok(Self {
            life: Shared::new(Life { slab: allocation }),
        })
    }
    fn projection(&self) -> BundleProjection<T> {
        BundleProjection(self.life.slab.clone())
    }
    fn alias(&self) -> Self {
        Self {
            life: self.life.clone(),
        }
    }
}
/// The clearable actual child payload token is independent of retained topology.
/// Holding this slot after root retirement retains no child payload-life token.
pub(super) struct BundleSlot<T: Body> {
    owner: Mutex<Option<BundleOwner<T>>>,
    projection: BundleProjection<T>,
}
impl<T: Body> BundleSlot<T> {
    pub(super) fn same_payload(&self, other: &Self) -> bool {
        std::ptr::eq(self.projection.0.get(), other.projection.0.get())
    }
    pub(super) fn new(owner: BundleOwner<T>) -> Self {
        let projection = owner.projection();
        Self {
            owner: Mutex::new(Some(owner)),
            projection,
        }
    }
    pub(super) fn projection(&self) -> BundleProjection<T> {
        self.projection.clone()
    }
    pub(super) fn original_credit(&self) -> &Shared<NativeCredit> {
        &self.projection.0.original
    }
    pub(super) fn body(&self) -> &T {
        &self.projection.0.body
    }
    pub(super) fn alias(&self) -> Self {
        let owner = self.owner.lock().expect("immutable bundle token");
        Self::new(owner.as_ref().expect("root payload token").alias())
    }
    pub(super) fn retire(&self) {
        let owner = self.owner.lock().expect("immutable bundle token").take();
        drop(owner);
    }
    pub(super) fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.projection.count(count)
    }
    pub(super) fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.projection.cleanup(permit);
    }
    pub(super) fn adopt(&self, publication: &mut Publication<'_>) {
        self.projection.adopt(publication);
    }
}
impl<T: Body> sealed::Source for BundleProjection<T> {}
impl<T: Body> OwnerGraph for BundleProjection<T> {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        if self.count_header(count)? {
            self.0.body.count(count)?;
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        if self.0.cell.clear_owned(permit) {
            self.0.credit.cleanup_for_count(permit);
            self.0.body.cleanup(permit);
            self.0.original.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        if publication.enter(&self.0.cell) {
            self.0.credit.adopt_under(publication);
            self.0.original.adopt(publication);
            self.0.body.adopt(publication);
        }
    }
}
impl<T: Body> BundleProjection<T> {
    pub(super) fn payload_retired(&self) -> bool {
        self.0.retired.load(Ordering::Acquire)
    }
    fn count_header(&self, count: &mut Count<'_, '_>) -> Result<bool, RetentionError> {
        let life = if self.0.retired.load(Ordering::Acquire) {
            0
        } else {
            crate::model::allocations::shared_inline_bytes::<Life<T>>()
        };
        let bytes = crate::model::allocations::shared_inline_bytes::<Allocation<T>>()
            .checked_add(life)
            .ok_or(RetentionError::Arithmetic)?;
        if count.enter(&self.0.cell, bytes)? {
            self.0.credit.validate_for_count(count)?;
            self.0.original.count(count)?;
            return Ok(true);
        }
        Ok(false)
    }
}
/// A freshly constructed typed component owns all fallible allocation liability.
pub(super) fn part<T: Body>(
    role: &ConstructionRole,
    original: &Shared<NativeCredit>,
    body: T,
) -> Result<BundleSlot<T>, PhysicalError> {
    Ok(BundleSlot::new(BundleOwner::build(
        body,
        SlabBuilder::new(role)?,
        original,
    )?))
}
