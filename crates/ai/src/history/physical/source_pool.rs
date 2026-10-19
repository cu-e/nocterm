//! Exclusive conversion splits original native credits into actual backing owners.
use super::slots::{NativeCredit, PhysicalError};
use crate::model::{
    json::{deferred::ledger::Reservation, lifetime::Shared},
    native::NativeOwned,
    retention::{
        CensusPermit, RetentionError,
        document::{ConstructionRole, PendingAllocation, Publication},
        graph::Count,
    },
};
use std::sync::{RwLock, RwLockReadGuard};

/// Never reachable from a published graph: only the converting owner can split.
pub(super) struct NativeSourcePool {
    remaining: Reservation,
    original: Shared<NativeCredit>,
}
impl NativeSourcePool {
    /// Header admission happens before the caller extracts its native payload.
    pub(super) fn new(
        role: &ConstructionRole,
        allocation: Reservation,
        header: PendingAllocation,
    ) -> Self {
        let original = NativeCredit::new(role, allocation.original_ledger(), header);
        Self {
            remaining: allocation,
            original,
        }
    }
    pub(super) fn original(&self) -> &Shared<NativeCredit> {
        &self.original
    }
    pub(super) fn remaining_bytes(&self) -> usize {
        self.remaining.bytes()
    }
    pub(super) fn split_backing(&mut self, bytes: usize) -> Result<Reservation, PhysicalError> {
        Ok(self.remaining.split(bytes)?)
    }
    pub(super) fn retained<T>(
        &mut self,
        value: T,
        bytes: usize,
    ) -> Result<RetainedBacking<T>, PhysicalError> {
        let receipt = self.split_backing(bytes)?;
        Ok(RetainedBacking {
            value,
            receipt: Some(receipt),
        })
    }
    #[expect(
        clippy::boxed_local,
        reason = "the Box is freed only after its replacement backing is paid"
    )]
    pub(super) fn unbox<T>(&mut self, value: Box<T>) -> Result<T, PhysicalError> {
        let receipt = self.split_backing(std::mem::size_of::<T>())?;
        // Moving out of the Box frees its allocation here.
        let value = *value;
        // The nested owner returned only after physically freeing its old Box.
        drop(receipt);
        Ok(value)
    }
}
/// Retained unchanged source storage physically drops before its original credit.
pub(super) struct RetainedBacking<T> {
    value: T,
    receipt: Option<Reservation>,
}
impl<T> RetainedBacking<T> {
    pub(super) fn from_slab(value: T) -> Self {
        Self {
            value,
            receipt: None,
        }
    }
    pub(super) fn get(&self) -> &T {
        &self.value
    }
    pub(super) fn count_metadata(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.receipt
            .as_ref()
            .map_or(Ok(()), |receipt| receipt.validate_pending(count))
    }
    pub(super) fn cleanup(&self, permit: &CensusPermit<'_>) {
        if let Some(receipt) = &self.receipt {
            receipt.cleanup_pending(permit);
        }
    }
    pub(super) fn adopt(&self, publication: &mut Publication<'_>) {
        if let Some(receipt) = &self.receipt {
            receipt.adopt_under(publication);
        }
    }
}
impl<T> std::ops::Deref for RetainedBacking<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.get()
    }
}
/// Clearable value precedes the zeroable receipt, which survives weak metadata.
pub(super) struct NativeBacking<T> {
    value: RwLock<Option<T>>,
    receipt: Option<Reservation>,
}
impl<T> NativeBacking<T> {
    pub(super) fn from_source(
        value: T,
        bytes: usize,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        Ok(Self {
            value: RwLock::new(Some(value)),
            receipt: Some(pool.split_backing(bytes)?),
        })
    }
    /// The surrounding actual SlabBuilder owns this newly allocated backing.
    pub(super) fn from_slab(value: T) -> Self {
        Self {
            value: RwLock::new(Some(value)),
            receipt: None,
        }
    }
    pub(super) fn read(&self) -> RwLockReadGuard<'_, Option<T>> {
        self.value.read().expect("immutable native backing")
    }
    pub(super) fn count_metadata(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.receipt
            .as_ref()
            .map_or(Ok(()), |receipt| receipt.validate_pending(count))
    }
    pub(super) fn cleanup(&self, permit: &CensusPermit<'_>) {
        if let Some(receipt) = &self.receipt {
            receipt.cleanup_pending(permit);
        }
    }
    pub(super) fn adopt(&self, publication: &mut Publication<'_>) {
        if let Some(receipt) = &self.receipt {
            receipt.adopt_under(publication);
        }
    }
    pub(super) fn retire(&self) {
        let value = self
            .value
            .write()
            .expect("immutable native backing retirement")
            .take();
        let _retire = RetireReceipt(self.receipt.as_ref());
        drop(value);
    }
}
struct RetireReceipt<'a>(Option<&'a Reservation>);
impl Drop for RetireReceipt<'_> {
    fn drop(&mut self) {
        if let Some(receipt) = self.0 {
            receipt.retire_after_physical();
        }
    }
}
/// IntoIter owns the exact old Vec allocation until all leftovers have dropped.
pub(super) struct SourceVector<T> {
    values: std::vec::IntoIter<T>,
    _receipt: Reservation,
}
impl<T> SourceVector<T> {
    pub(super) fn new(values: Vec<T>, pool: &mut NativeSourcePool) -> Result<Self, PhysicalError> {
        let bytes = values
            .capacity()
            .checked_mul(std::mem::size_of::<T>())
            .ok_or(RetentionError::Arithmetic)?;
        let receipt = pool.split_backing(bytes)?;
        Ok(Self {
            values: values.into_iter(),
            _receipt: receipt,
        })
    }
    pub(super) fn next(&mut self) -> Option<T> {
        self.values.next()
    }
}

/// Payload is declared before the exclusive pool for every refusal/unwind path.
pub(super) struct NativeSource<T> {
    value: Option<T>,
    pool: NativeSourcePool,
}
impl<T> NativeSource<T> {
    pub(super) fn new(
        value: NativeOwned<T>,
        role: &ConstructionRole,
    ) -> Result<Self, PhysicalError> {
        if !value.ledger().belongs_to_document(role.scope()) {
            return Err(RetentionError::ForeignAuthority.into());
        }
        let header = NativeCredit::reserve_header(role)?;
        let (value, allocation) = value.into_parts();
        Ok(Self {
            value: Some(value),
            pool: NativeSourcePool::new(role, allocation, header),
        })
    }
    pub(super) fn original(&self) -> &Shared<NativeCredit> {
        self.pool.original()
    }
    pub(super) fn take(&mut self) -> T {
        self.value
            .take()
            .expect("exclusive native source consumed once")
    }
    pub(super) fn pool(&mut self) -> &mut NativeSourcePool {
        &mut self.pool
    }
}

#[cfg(test)]
mod tests;
