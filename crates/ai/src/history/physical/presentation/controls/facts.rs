//! Whole immutable native control facts retain their exact original backing receipts.
use super::super::super::{
    bundle::{self, Body, BundleSlot},
    slots::{NativeSource, PayloadGuard, PhysicalError},
    source_pool::NativeBacking,
};
use crate::model::{
    native::NativeOwned,
    retention::{
        CensusPermit, RetentionError,
        document::{ConstructionRole, Publication},
        graph::Count,
    },
};
mod payload;
use payload::ControlPayload;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) struct ControlFacts<T> {
    value: NativeBacking<T>,
    bytes: AtomicUsize,
    policy: AtomicUsize,
}
impl<T: ControlPayload> ControlFacts<T> {
    pub(super) fn prepare(
        role: &ConstructionRole,
        value: NativeOwned<T>,
        policy: usize,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut source = NativeSource::new(value, role)?;
        let value = source.take();
        Self::from_source(role, value, source.pool(), policy)
    }
    pub(super) fn from_source(
        role: &ConstructionRole,
        value: T,
        pool: &mut super::super::super::slots::NativeSourcePool,
        policy: usize,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let original = pool.original().clone();
        let bytes = pool.remaining_bytes();
        let value = NativeBacking::from_source(value, bytes, pool)?;
        bundle::part(
            role,
            &original,
            Self {
                value,
                bytes: AtomicUsize::new(bytes),
                policy: AtomicUsize::new(policy),
            },
        )
    }
    pub(super) fn read(&self) -> PayloadGuard<'_, T> {
        PayloadGuard(self.value.read())
    }
}
impl<T: ControlPayload> Body for ControlFacts<T> {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        let bytes = self.bytes.load(Ordering::Acquire);
        self.value.count_metadata(count)?;
        if let Some(value) = self.value.read().as_ref() {
            value.count_children(count)?;
        }
        count.record_policy(Some(self.policy.load(Ordering::Acquire)))?;
        count.add_exclusive(bytes)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.value.cleanup(permit);
        if let Some(value) = self.value.read().as_ref() {
            value.cleanup_children(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.value.adopt(publication);
        if let Some(value) = self.value.read().as_ref() {
            value.adopt_children(publication);
        }
    }
    fn retire(&self) {
        self.value.retire();
        self.bytes.store(0, Ordering::Release);
        self.policy.store(0, Ordering::Release);
    }
}
