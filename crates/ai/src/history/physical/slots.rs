//! Real payload slots and prepaid typed slab allocation; no descriptor vectors.
use super::source_pool::{NativeBacking, SourceVector};
pub(super) use super::source_pool::{NativeSource, NativeSourcePool};
use crate::model::{
    extensions::{Extensions, ExtensionsIdentity},
    json::{JsonReceipt, OwnedJson, deferred::DecodeLedger},
    retention::{
        CensusPermit, RetentionError,
        document::{AllocationPlan, ConstructionRole, PendingAllocation, Publication},
        graph::{Count, OwnerGraph},
    },
};
use std::{
    ops::Deref,
    sync::{
        RwLock, RwLockReadGuard,
        atomic::{AtomicBool, Ordering},
    },
};

pub struct TextSlot {
    value: NativeBacking<String>,
    capacity: usize,
    retired: AtomicBool,
}
impl TextSlot {
    pub(super) fn new(value: String, pool: &mut NativeSourcePool) -> Result<Self, PhysicalError> {
        let capacity = value.capacity();
        Ok(Self {
            value: NativeBacking::from_source(value, capacity, pool)?,
            capacity,
            retired: AtomicBool::new(false),
        })
    }
    pub(super) fn from_slab(value: String) -> Self {
        Self {
            capacity: value.capacity(),
            value: NativeBacking::from_slab(value),
            retired: AtomicBool::new(false),
        }
    }
    pub fn read(&self) -> TextGuard<'_> {
        TextGuard(self.value.read())
    }
    pub(super) fn bytes(&self) -> usize {
        if self.retired.load(Ordering::Acquire) {
            0
        } else {
            self.capacity
        }
    }
    pub(super) fn retire(&self) {
        self.value.retire();
        self.retired.store(true, Ordering::Release);
    }
    pub(super) fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.value.count_metadata(count)?;
        count.add_exclusive(self.bytes())
    }
    pub(super) fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.value.cleanup(permit);
    }
    pub(super) fn adopt(&self, publication: &mut Publication<'_>) {
        self.value.adopt(publication);
    }
}
pub struct TextGuard<'a>(RwLockReadGuard<'a, Option<String>>);
impl TextGuard<'_> {
    pub(crate) fn capacity(&self) -> usize {
        self.0
            .as_ref()
            .expect("real owner pins text payload")
            .capacity()
    }
}
impl Deref for TextGuard<'_> {
    type Target = str;
    fn deref(&self) -> &str {
        self.0.as_deref().expect("real owner pins text payload")
    }
}
pub(super) struct JsonSlot {
    value: RwLock<Option<OwnedJson>>,
    receipt: JsonReceipt,
}
impl JsonSlot {
    pub(super) fn new(value: OwnedJson) -> Self {
        let receipt = value.receipt().clone();
        Self {
            value: RwLock::new(Some(value)),
            receipt,
        }
    }
    pub(super) fn retire(&self) {
        let value = self.value.write().expect("immutable JSON slot").take();
        drop(value);
    }
    pub(super) fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.receipt.count(count)
    }
    pub(super) fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.receipt.cleanup(permit);
    }
    pub(super) fn adopt(&self, publication: &mut Publication<'_>) {
        self.receipt.adopt(publication);
    }
}
pub struct ExtensionSlot {
    value: RwLock<Option<Extensions>>,
    receipt: ExtensionsIdentity,
}
impl ExtensionSlot {
    pub(super) fn new(value: Extensions) -> Self {
        let receipt = value.identity();
        Self {
            value: RwLock::new(Some(value)),
            receipt,
        }
    }
    pub(super) fn retire(&self) {
        let value = self.value.write().expect("immutable extension slot").take();
        drop(value);
    }
    pub(super) fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.receipt.count(count)
    }
    pub(super) fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.receipt.cleanup(permit);
    }
    pub(super) fn adopt(&self, publication: &mut Publication<'_>) {
        self.receipt.adopt(publication);
    }
}
/// Only the owner constructing the real Vec may derive its capacity request.
/// The aggregate credit is an actual pending token, not a vector of hints.
pub(super) struct SlabBuilder<'a> {
    role: &'a ConstructionRole,
    credit: PendingAllocation,
}
impl<'a> SlabBuilder<'a> {
    pub(super) fn new(role: &'a ConstructionRole) -> Result<Self, RetentionError> {
        Ok(Self {
            role,
            credit: role.reserve_pending(AllocationPlan::text(0))?,
        })
    }
    pub(super) fn role(&self) -> &'a ConstructionRole {
        self.role
    }
    pub(super) fn vector<T>(&mut self, capacity: usize) -> Result<Vec<T>, PhysicalError> {
        let mut pending = self
            .role
            .reserve_pending(AllocationPlan::array::<T>(capacity)?)?;
        let mut values = Vec::new();
        values.try_reserve_exact(capacity)?;
        if std::mem::size_of::<T>() != 0 && values.capacity() != capacity {
            return Err(PhysicalError::CapacityProfile);
        }
        self.credit.merge_from(&mut pending)?;
        Ok(values)
    }
    /// Prepays an exact text buffer while every source text is still live.
    pub(super) fn text(&mut self, capacity: usize) -> Result<String, PhysicalError> {
        let mut pending = self.role.reserve_pending(AllocationPlan::text(capacity))?;
        let mut value = String::new();
        value.try_reserve_exact(capacity)?;
        if value.capacity() != capacity {
            return Err(PhysicalError::CapacityProfile);
        }
        self.credit.merge_from(&mut pending)?;
        Ok(value)
    }
    pub(super) fn exclusive_box<T>(&mut self) -> Result<(), RetentionError> {
        let mut pending = self.role.reserve_pending(AllocationPlan::array::<T>(1)?)?;
        self.credit.merge_from(&mut pending)
    }
    pub(super) fn shared<T>(&mut self) -> Result<(), RetentionError> {
        let mut pending = self.role.reserve_pending(AllocationPlan::shared::<T>())?;
        self.credit.merge_from(&mut pending)
    }
    pub(super) fn map_in<U, T>(
        &mut self,
        values: Vec<U>,
        pool: &mut NativeSourcePool,
        mut convert: impl FnMut(U, &mut Self, &mut NativeSourcePool) -> Result<T, PhysicalError>,
    ) -> Result<Vec<T>, PhysicalError> {
        let len = values.len();
        let mut source = SourceVector::new(values, pool)?;
        let mut result = self.vector(len)?;
        while let Some(value) = source.next() {
            result.push(convert(value, self, pool)?);
        }
        drop(source);
        Ok(result)
    }
    pub(super) fn finish(self) -> PendingAllocation {
        self.credit
    }
}
#[derive(Debug, thiserror::Error)]
pub enum PhysicalError {
    #[error(transparent)]
    Measurement(#[from] crate::history::MeasurementError),
    #[error("queued prompt limit exceeded")]
    QueueLimit,
    #[error("queued prompt was already dispatched")]
    QueuedPromptMissing,
    #[error("Imported queued images require managed preparation before editing")]
    QueuedPromptNeedsPreparation,
    #[error(transparent)]
    ImageCollection(#[from] crate::images::ImageCollectionError),
    #[error("physical document admission failed: {0}")]
    Retention(#[from] RetentionError),
    #[error("physical document allocation failed: {0}")]
    Allocation(#[from] std::collections::TryReserveError),
    #[error("controlled native construction failed: {0}")]
    Native(#[from] crate::model::json::deferred::JsonDecodeError),
    #[error("physical allocation capacity profile is unsupported")]
    CapacityProfile,
}
/// Inline ownership fact from the actual controlled value. Kept with the actual
/// slot so provenance and local diagnostic credit survive every consuming move.
pub(super) struct NativeCredit {
    ledger: DecodeLedger,
    cell: crate::model::retention::RetentionCell,
    credit: PendingAllocation,
}
impl NativeCredit {
    pub(super) fn reserve_header(
        role: &ConstructionRole,
    ) -> Result<PendingAllocation, RetentionError> {
        role.reserve_pending(AllocationPlan::shared::<Self>())
    }
    pub(super) fn new(
        role: &ConstructionRole,
        ledger: &DecodeLedger,
        credit: PendingAllocation,
    ) -> crate::model::json::lifetime::Shared<Self> {
        let ledger = ledger.share();
        let cell = crate::model::retention::RetentionCell::new(role.scope().authority());
        crate::model::json::lifetime::Shared::new(Self {
            ledger,
            cell,
            credit,
        })
    }
    pub(super) fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        if count.enter(
            &self.cell,
            crate::model::allocations::shared_inline_bytes::<Self>(),
        )? {
            self.credit.validate_for_count(count)?;
            self.ledger.count(count)?;
        }
        Ok(())
    }
    pub(super) fn cleanup(&self, permit: &CensusPermit<'_>) {
        if self.cell.clear_owned(permit) {
            self.credit.cleanup_for_count(permit);
            self.ledger.cleanup(permit);
        }
    }
    pub(super) fn adopt(&self, publication: &mut Publication<'_>) {
        if publication.enter(&self.cell) {
            self.credit.adopt_under(publication);
            self.ledger.adopt(publication);
        }
    }
}
pub(super) fn optional_text(
    value: Option<String>,
    pool: &mut NativeSourcePool,
) -> Result<Option<TextSlot>, PhysicalError> {
    value.map(|value| TextSlot::new(value, pool)).transpose()
}
pub(super) fn retire_text(value: &Option<TextSlot>) {
    if let Some(value) = value {
        value.retire();
    }
}
pub(super) fn vector_bytes<T>(value: &Vec<T>) -> Result<usize, RetentionError> {
    value
        .capacity()
        .checked_mul(std::mem::size_of::<T>())
        .ok_or(RetentionError::Arithmetic)
}

pub struct PathSlot {
    value: NativeBacking<std::path::PathBuf>,
    capacity: usize,
    retired: AtomicBool,
}
impl PathSlot {
    pub(super) fn new(
        value: std::path::PathBuf,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        let capacity = value.capacity();
        Ok(Self {
            value: NativeBacking::from_source(value, capacity, pool)?,
            capacity,
            retired: AtomicBool::new(false),
        })
    }
    pub fn read(&self) -> PathGuard<'_> {
        PathGuard(self.value.read())
    }
    pub(super) fn bytes(&self) -> usize {
        if self.retired.load(Ordering::Acquire) {
            0
        } else {
            self.capacity
        }
    }
    pub(super) fn retire(&self) {
        self.value.retire();
        self.retired.store(true, Ordering::Release);
    }
    pub(super) fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.value.count_metadata(count)?;
        count.add_exclusive(self.bytes())
    }
    pub(super) fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.value.cleanup(permit);
    }
    pub(super) fn adopt(&self, publication: &mut Publication<'_>) {
        self.value.adopt(publication);
    }
}
pub struct PathGuard<'a>(RwLockReadGuard<'a, Option<std::path::PathBuf>>);
impl PathGuard<'_> {
    pub(crate) fn capacity(&self) -> usize {
        self.0.as_ref().expect("real owner pins path").capacity()
    }
}
impl Deref for PathGuard<'_> {
    type Target = std::path::Path;
    fn deref(&self) -> &Self::Target {
        self.0.as_deref().expect("real owner pins path")
    }
}

pub(super) struct IdSlot<T> {
    value: RwLock<Option<T>>,
    receipt: crate::model::ids::IdentifierIdentity,
}
impl IdSlot<crate::model::ids::SessionId> {
    pub(super) fn session(value: crate::model::ids::SessionId) -> Self {
        let receipt = value.retention_identity();
        Self {
            value: RwLock::new(Some(value)),
            receipt,
        }
    }
}
impl IdSlot<crate::model::ids::ToolId> {
    pub(super) fn tool(value: crate::model::ids::ToolId) -> Self {
        let receipt = value.retention_identity();
        Self {
            value: RwLock::new(Some(value)),
            receipt,
        }
    }
}
impl IdSlot<crate::model::ids::ProviderTerminalId> {
    pub(super) fn terminal(value: crate::model::ids::ProviderTerminalId) -> Self {
        let receipt = value.retention_identity();
        Self {
            value: RwLock::new(Some(value)),
            receipt,
        }
    }
}
impl IdSlot<crate::model::ids::ModeId> {
    pub(super) fn mode(value: crate::model::ids::ModeId) -> Self {
        let receipt = value.retention_identity();
        Self {
            value: RwLock::new(Some(value)),
            receipt,
        }
    }
}
impl IdSlot<crate::model::ids::ConfigId> {
    pub(super) fn config(value: crate::model::ids::ConfigId) -> Self {
        let receipt = value.retention_identity();
        Self {
            value: RwLock::new(Some(value)),
            receipt,
        }
    }
}
impl IdSlot<crate::model::ids::ConfigChoiceId> {
    pub(super) fn choice(value: crate::model::ids::ConfigChoiceId) -> Self {
        let receipt = value.retention_identity();
        Self {
            value: RwLock::new(Some(value)),
            receipt,
        }
    }
}
impl IdSlot<crate::model::ids::ConfigGroupId> {
    pub(super) fn group(value: crate::model::ids::ConfigGroupId) -> Self {
        let receipt = value.retention_identity();
        Self {
            value: RwLock::new(Some(value)),
            receipt,
        }
    }
}
impl<T> IdSlot<T> {
    pub(super) fn read(&self) -> PayloadGuard<'_, T> {
        PayloadGuard(self.value.read().expect("immutable ID slot"))
    }
    pub(super) fn retire(&self) {
        let value = self.value.write().expect("immutable ID slot").take();
        drop(value);
    }
    pub(super) fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.receipt.count(count)
    }
    pub(super) fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.receipt.cleanup(permit);
    }
    pub(super) fn adopt(&self, publication: &mut Publication<'_>) {
        self.receipt.adopt(publication);
    }
}
pub struct PayloadGuard<'a, T>(pub(super) RwLockReadGuard<'a, Option<T>>);
impl<T> Deref for PayloadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0.as_ref().expect("real owner pins payload")
    }
}
impl JsonSlot {
    pub(super) fn read(&self) -> PayloadGuard<'_, OwnedJson> {
        PayloadGuard(self.value.read().expect("immutable JSON slot"))
    }
}
impl ExtensionSlot {
    pub fn read(&self) -> PayloadGuard<'_, Extensions> {
        PayloadGuard(self.value.read().expect("immutable extension slot"))
    }
}

pub(super) struct DisplaySlot {
    value: RwLock<Option<crate::model::shared_display::SharedDisplay>>,
    receipt: crate::model::shared_display::DisplayIdentity,
}
impl DisplaySlot {
    pub(super) fn new(value: crate::model::shared_display::SharedDisplay) -> Self {
        let receipt = value.identity();
        Self {
            value: RwLock::new(Some(value)),
            receipt,
        }
    }
    pub(super) fn read(&self) -> PayloadGuard<'_, crate::model::shared_display::SharedDisplay> {
        PayloadGuard(self.value.read().expect("immutable display slot"))
    }
    pub(super) fn retire(&self) {
        let value = self.value.write().expect("immutable display slot").take();
        drop(value);
    }
    pub(super) fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.receipt.count(count)
    }
    pub(super) fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.receipt.cleanup(permit);
    }
    pub(super) fn adopt(&self, publication: &mut Publication<'_>) {
        self.receipt.adopt(publication);
    }
}

impl super::bundle::Body for TextSlot {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        Self::count(self, count)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        Self::cleanup(self, permit);
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        Self::adopt(self, publication);
    }
    fn retire(&self) {
        Self::retire(self);
    }
}
/// Receipt-backed slots become independently aliasable bundle bodies.
macro_rules! receipt_body {
    ($($slot:ty),+) => {$(
        impl super::bundle::Body for $slot {
            fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
                Self::count(self, count)
            }
            fn cleanup(&self, permit: &CensusPermit<'_>) {
                Self::cleanup(self, permit);
            }
            fn adopt(&self, publication: &mut Publication<'_>) {
                Self::adopt(self, publication);
            }
            fn retire(&self) {
                Self::retire(self);
            }
        }
    )+};
}
receipt_body!(
    JsonSlot,
    ExtensionSlot,
    DisplaySlot,
    IdSlot<crate::model::ids::ToolId>,
    IdSlot<crate::model::ids::SessionId>,
    IdSlot<crate::model::ids::ModeId>,
    IdSlot<crate::model::ids::ConfigId>,
    IdSlot<crate::model::ids::ConfigChoiceId>,
    IdSlot<crate::model::ids::ConfigGroupId>
);
