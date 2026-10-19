//! Real root lifetime and closed controlled construction/adoption boundary.
use super::{
    bundle::{self, BundleOwner, BundleSlot},
    composer::ComposerSlot,
    composition::ComposerGraph,
    framing::{MetadataSlot, ResumeSlot},
    inputs::LiveInputs,
    presentation::LivePresentation,
    rows::{RowSlot, TranscriptSlot},
    slots::*,
};
use crate::{
    history::{
        ChatMetadata, ComposerData, PreparedHistory, PreparedProjection, ResumeDescriptor,
        TranscriptData,
    },
    model::{
        json::lifetime::Shared,
        native::NativeOwned,
        retention::{
            CensusPermit, RetentionCell, RetentionError,
            document::{
                ConstructionRole, DocumentRetention, PendingAllocation, Publication, RetainedRole,
            },
            graph::{self, Count, OwnerGraph, sealed},
        },
    },
};
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
};

/// Fixed typed edges to actual independently shared component storage.
struct SnapshotParts {
    id: BundleSlot<TextSlot>,
    metadata: BundleSlot<MetadataSlot>,
    resume: BundleSlot<ResumeSlot>,
    transcript: BundleSlot<TranscriptSlot>,
    composition: ComposerGraph,
}
impl SnapshotParts {
    fn alias(&self) -> Self {
        Self {
            id: self.id.alias(),
            metadata: self.metadata.alias(),
            resume: self.resume.alias(),
            transcript: self.transcript.alias(),
            composition: self.composition.alias(),
        }
    }
    fn retire(&self) {
        self.id.retire();
        self.metadata.retire();
        self.resume.retire();
        self.transcript.retire();
        self.composition.retire();
    }
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.id.count(count)?;
        self.metadata.count(count)?;
        self.resume.count(count)?;
        self.transcript.count(count)?;
        self.composition.count(count)?;
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.id.cleanup(permit);
        self.metadata.cleanup(permit);
        self.resume.cleanup(permit);
        self.transcript.cleanup(permit);
        self.composition.cleanup(permit);
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.id.adopt(publication);
        self.metadata.adopt(publication);
        self.resume.adopt(publication);
        self.transcript.adopt(publication);
        self.composition.adopt(publication);
    }
}
struct SnapshotAllocation {
    parts: SnapshotParts,
    prepared: Mutex<Option<PreparedHistory>>,
    prepared_projection: Option<PreparedProjection>,
    cell: RetentionCell,
    payload_retired: AtomicBool,
    policy: OnceLock<super::policy::SnapshotPolicy>,
    credit: PendingAllocation,
}
struct PayloadLife {
    slab: Shared<SnapshotAllocation>,
}
impl Drop for PayloadLife {
    fn drop(&mut self) {
        // Shared::into_inner freed the lifetime block before this sealed cleanup.
        self.slab.parts.retire();
        let prepared = self
            .slab
            .prepared
            .lock()
            .expect("immutable prepared slot")
            .take();
        drop(prepared);
        self.slab.payload_retired.store(true, Ordering::Release);
    }
}
pub struct SnapshotOwner {
    life: Shared<PayloadLife>,
}
/// Immutable access to the actual prepared cache while the physical owner lives.
pub struct PreparedGuard<'a>(std::sync::MutexGuard<'a, Option<PreparedHistory>>);
impl std::ops::Deref for PreparedGuard<'_> {
    type Target = Option<PreparedHistory>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
#[derive(Clone)]
pub(crate) struct SnapshotProjection(Shared<SnapshotAllocation>);
impl SnapshotOwner {
    pub(crate) fn policy_bytes(&self) -> Result<usize, RetentionError> {
        self.life
            .slab
            .policy
            .get()
            .map(super::policy::SnapshotPolicy::bytes)
            .ok_or(RetentionError::UncertifiedPolicy)
    }
    pub(crate) fn authority(&self) -> &crate::model::retention::RetentionAuthority {
        self.life.slab.cell.authority()
    }
    pub(crate) fn retained_bytes(&self) -> Result<usize, RetentionError> {
        graph::measure(self.authority(), &self.projection())
    }
    pub(crate) fn transcript_policy_capacities(&self) -> (usize, usize) {
        self.life.slab.parts.transcript.body().policy_capacities()
    }
    pub fn prepared(&self) -> PreparedGuard<'_> {
        PreparedGuard(
            self.life
                .slab
                .prepared
                .lock()
                .expect("immutable prepared slot"),
        )
    }
    pub fn id(&self) -> super::TextGuard<'_> {
        self.life.slab.parts.id.body().read()
    }
    pub fn metadata(&self) -> &MetadataSlot {
        self.life.slab.parts.metadata.body()
    }
    pub fn resume(&self) -> &ResumeSlot {
        self.life.slab.parts.resume.body()
    }
    pub fn rows(&self) -> &[RowSlot] {
        self.life.slab.parts.transcript.body().rows()
    }
    pub fn times(&self) -> &[u64] {
        self.life.slab.parts.transcript.body().times()
    }
    pub fn composer(&self) -> &ComposerSlot {
        self.life.slab.parts.composition.composer()
    }
    pub fn live_inputs(&self) -> Option<&LiveInputs> {
        self.life.slab.parts.composition.inputs()
    }
    pub fn presentation(&self) -> Option<&LivePresentation> {
        self.life.slab.parts.composition.presentation()
    }
    pub(crate) fn projection(&self) -> SnapshotProjection {
        SnapshotProjection(self.life.slab.clone())
    }
    pub(crate) fn clone_within_role(&self) -> Self {
        Self {
            life: self.life.clone(),
        }
    }
}
/// The actual operation borrow prevents dropping/clearing its registered source
/// before aliases are adopted. Payload precedes the strong account pin.
pub struct SnapshotStaging<'a> {
    owner: SnapshotOwner,
    scope: DocumentRetention,
    _operation: &'a ConstructionRole,
}
impl SnapshotStaging<'_> {
    pub(in crate::history) fn for_dispatch(
        owner: SnapshotOwner,
        operation: &ConstructionRole,
    ) -> SnapshotStaging<'_> {
        SnapshotStaging {
            owner,
            scope: operation.scope().clone(),
            _operation: operation,
        }
    }
    pub(crate) fn snapshot(&self) -> &SnapshotOwner {
        &self.owner
    }
    pub(crate) fn check_scope(&self, scope: &DocumentRetention) -> Result<(), RetentionError> {
        if !self.scope.same_document(scope) {
            return Err(RetentionError::ForeignAuthority);
        }
        graph::measure_document(scope, &self.projection())?;
        Ok(())
    }
    pub(crate) fn projection(&self) -> SnapshotProjection {
        self.owner.projection()
    }
    pub(crate) fn into_owner(self) -> SnapshotOwner {
        self.owner
    }
}
mod builder;
pub use builder::SnapshotBuilder;

impl sealed::Source for SnapshotProjection {}
impl OwnerGraph for SnapshotProjection {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        let retired = self.0.payload_retired.load(Ordering::Acquire);
        count.record_policy(if retired {
            Some(0)
        } else {
            self.0
                .policy
                .get()
                .map(super::policy::SnapshotPolicy::bytes)
        })?;
        let token = if self.0.payload_retired.load(Ordering::Acquire) {
            0
        } else {
            crate::model::allocations::shared_inline_bytes::<PayloadLife>()
        };
        let bytes = crate::model::allocations::shared_inline_bytes::<SnapshotAllocation>()
            .checked_add(token)
            .ok_or(RetentionError::Arithmetic)?;
        if count.enter(&self.0.cell, bytes)? {
            self.0.credit.validate_for_count(count)?;
            self.0.parts.count(count)?;
            if let Some(prepared) = &self.0.prepared_projection {
                prepared.count(count)?;
            }
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        if self.0.cell.clear_owned(permit) {
            self.0.credit.cleanup_for_count(permit);
            self.0.parts.cleanup(permit);
            if let Some(prepared) = &self.0.prepared_projection {
                prepared.cleanup(permit);
            }
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        if publication.enter(&self.0.cell) {
            self.0.credit.adopt_under(publication);
            self.0.parts.adopt(publication);
            if let Some(prepared) = &self.0.prepared_projection {
                prepared.adopt(publication);
            }
        }
    }
}
