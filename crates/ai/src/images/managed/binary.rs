//! Original inspected binary Arc with authentic prepaid lifetime and weak metadata.
use crate::model::{
    allocations::shared_inline_bytes,
    json::{
        deferred::{DecodeLedger, JsonDecodeError, ledger::Reservation},
        lifetime::Shared,
    },
    native::NativeOwned,
    retention::{
        CensusPermit, RetentionCell, RetentionError,
        document::Publication,
        graph::{Count, OwnerGraph, sealed},
    },
};
use std::{
    alloc::Layout,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

struct Metadata {
    cell: RetentionCell,
    retired: AtomicBool,
    header: Reservation,
    payload: Reservation,
    life: Reservation,
}
struct Life {
    bytes: Option<Arc<[u8]>>,
    metadata: Shared<Metadata>,
}
impl Drop for Life {
    fn drop(&mut self) {
        drop(self.bytes.take());
        self.metadata.retired.store(true, Ordering::Release);
        self.metadata.payload.retire_after_physical();
        self.metadata.life.retire_after_physical();
    }
}
/// The original inspected allocation. Callers can borrow bytes but cannot
/// detach an Arc, Weak or binary owner from the inspection operation.
pub struct ManagedImageBytes {
    life: Shared<Life>,
}
#[derive(Clone)]
pub(crate) struct BinaryProjection(Shared<Metadata>);
impl ManagedImageBytes {
    pub(crate) fn from_native(
        ledger: &DecodeLedger,
        bytes: NativeOwned<Vec<u8>>,
    ) -> Result<Self, JsonDecodeError> {
        // Joining an inline fact authenticates the exact ledger before conversion.
        let bytes = bytes.join(NativeOwned::scalar((), ledger)?)?;
        let header = ledger.reserve(shared_inline_bytes::<Metadata>())?;
        let life = ledger.reserve(shared_inline_bytes::<Life>())?;
        let payload = ledger.reserve(arc_bytes(bytes.get().0.len())?)?;
        let ((bytes, ()), allocation) = bytes.into_parts();
        let bytes: Arc<[u8]> = bytes.into();
        // Vec->Arc physically retired the original vector before its old credit.
        drop(allocation);
        let metadata = Shared::new(Metadata {
            cell: RetentionCell::new(ledger.authority()),
            retired: AtomicBool::new(false),
            header,
            payload,
            life,
        });
        Ok(Self {
            life: Shared::new(Life {
                bytes: Some(bytes),
                metadata,
            }),
        })
    }
    pub fn bytes(&self) -> &[u8] {
        self.life.bytes.as_deref().expect("original binary owner")
    }
    pub fn same_allocation(&self, other: &Self) -> bool {
        std::ptr::eq(self.life.metadata.get(), other.life.metadata.get())
    }
    pub(crate) fn projection(&self) -> BinaryProjection {
        BinaryProjection(self.life.metadata.clone())
    }
}
fn arc_bytes(length: usize) -> Result<usize, JsonDecodeError> {
    let header = Layout::new::<[AtomicUsize; 2]>();
    let bytes = Layout::array::<u8>(length).map_err(|_| JsonDecodeError::Arithmetic)?;
    header
        .extend(bytes)
        .map(|(layout, _)| layout.pad_to_align().size())
        .map_err(|_| JsonDecodeError::Arithmetic)
}
impl sealed::Source for BinaryProjection {}
impl OwnerGraph for BinaryProjection {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        let live = !self.0.retired.load(Ordering::Acquire);
        let payload = if live {
            self.0
                .payload
                .bytes()
                .checked_add(shared_inline_bytes::<Life>())
                .ok_or(RetentionError::Arithmetic)?
        } else {
            0
        };
        let bytes = shared_inline_bytes::<Metadata>()
            .checked_add(payload)
            .ok_or(RetentionError::Arithmetic)?;
        if count.enter(&self.0.cell, bytes)? {
            self.0.header.validate_pending(count)?;
            self.0.payload.validate_pending(count)?;
            self.0.life.validate_pending(count)?;
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        if self.0.cell.clear_owned(permit) {
            self.0.header.cleanup_pending(permit);
            self.0.payload.cleanup_pending(permit);
            self.0.life.cleanup_pending(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        if publication.enter(&self.0.cell) {
            self.0.header.adopt_under(publication);
            self.0.payload.adopt_under(publication);
            self.0.life.adopt_under(publication);
        }
    }
}
