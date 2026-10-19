//! Linear capabilities for actual feature, Store and delivery producers.
use super::{
    AllocationPlan, DocumentRetention, PendingAllocation, RetainedPayload, RetainedProjection,
    RetentionError,
};
use crate::{
    history::physical::{SnapshotOwner, SnapshotProjection, SnapshotStaging},
    model::retention::{
        CensusPermit,
        graph::{Count, OwnerGraph, sealed},
    },
};
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RetainedKind {
    Live,
    Candidate,
    WriterActive,
    WriterLatest,
    Delivery,
    PromptDispatch,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ConstructionKind {
    Foreground,
    Writer,
    Reader,
    Source,
}
#[derive(Default)]
pub(super) struct RetainedSlot {
    current: Option<RetainedProjection>,
    retiring: Option<RetainedProjection>,
}
impl RetainedSlot {
    pub(super) fn replace(&mut self, next: Option<RetainedProjection>) {
        assert!(self.retiring.is_none(), "synchronous producer retirement");
        self.retiring = self.current.take();
        self.current = next;
    }
    pub(super) fn take_retiring(&mut self) -> Option<RetainedProjection> {
        self.retiring.take()
    }
    pub(super) fn retiring(&self) -> Option<&RetainedProjection> {
        self.retiring.as_ref()
    }
}
pub(super) struct RetainedRoles {
    live: RetainedSlot,
    candidate: RetainedSlot,
    writer_active: RetainedSlot,
    writer_latest: RetainedSlot,
    delivery: RetainedSlot,
    prompt_dispatch: RetainedSlot,
    foreground: Option<SnapshotProjection>,
    writer: Option<SnapshotProjection>,
    reader: Option<SnapshotProjection>,
    source: Option<SnapshotProjection>,
}
impl RetainedRoles {
    pub(super) fn new() -> Self {
        Self {
            live: RetainedSlot::default(),
            candidate: RetainedSlot::default(),
            writer_active: RetainedSlot::default(),
            writer_latest: RetainedSlot::default(),
            delivery: RetainedSlot::default(),
            prompt_dispatch: RetainedSlot::default(),
            foreground: None,
            writer: None,
            reader: None,
            source: None,
        }
    }
    pub(super) fn retained_mut(&mut self, role: RetainedKind) -> &mut RetainedSlot {
        match role {
            RetainedKind::Live => &mut self.live,
            RetainedKind::Candidate => &mut self.candidate,
            RetainedKind::WriterActive => &mut self.writer_active,
            RetainedKind::WriterLatest => &mut self.writer_latest,
            RetainedKind::Delivery => &mut self.delivery,
            RetainedKind::PromptDispatch => &mut self.prompt_dispatch,
        }
    }
    pub(super) fn construction_mut(
        &mut self,
        role: ConstructionKind,
    ) -> &mut Option<SnapshotProjection> {
        match role {
            ConstructionKind::Foreground => &mut self.foreground,
            ConstructionKind::Writer => &mut self.writer,
            ConstructionKind::Reader => &mut self.reader,
            ConstructionKind::Source => &mut self.source,
        }
    }
    fn retained(&self) -> [&RetainedSlot; 6] {
        [
            &self.live,
            &self.candidate,
            &self.writer_active,
            &self.writer_latest,
            &self.delivery,
            &self.prompt_dispatch,
        ]
    }
    fn construction(&self) -> [&Option<SnapshotProjection>; 4] {
        [&self.foreground, &self.writer, &self.reader, &self.source]
    }
    pub(super) fn retained_slot(&self, kind: RetainedKind) -> &RetainedSlot {
        match kind {
            RetainedKind::Live => &self.live,
            RetainedKind::Candidate => &self.candidate,
            RetainedKind::WriterActive => &self.writer_active,
            RetainedKind::WriterLatest => &self.writer_latest,
            RetainedKind::Delivery => &self.delivery,
            RetainedKind::PromptDispatch => &self.prompt_dispatch,
        }
    }
    pub(super) fn construction_slot(&self, kind: ConstructionKind) -> Option<&SnapshotProjection> {
        match kind {
            ConstructionKind::Foreground => self.foreground.as_ref(),
            ConstructionKind::Writer => self.writer.as_ref(),
            ConstructionKind::Reader => self.reader.as_ref(),
            ConstructionKind::Source => self.source.as_ref(),
        }
    }
    pub(super) fn count_except(
        &self,
        retiring: Option<RetainedKind>,
        source: Option<ConstructionKind>,
        count: &mut Count<'_, '_>,
    ) -> Result<(), RetentionError> {
        for kind in [
            RetainedKind::Live,
            RetainedKind::Candidate,
            RetainedKind::WriterActive,
            RetainedKind::WriterLatest,
            RetainedKind::Delivery,
            RetainedKind::PromptDispatch,
        ] {
            let slot = self.retained_slot(kind);
            if let Some(current) = &slot.current {
                current.count(count)?;
            }
            if retiring != Some(kind)
                && let Some(old) = &slot.retiring
            {
                old.count(count)?;
            }
        }
        for kind in [
            ConstructionKind::Foreground,
            ConstructionKind::Writer,
            ConstructionKind::Reader,
            ConstructionKind::Source,
        ] {
            if source != Some(kind)
                && let Some(projection) = self.construction_slot(kind)
            {
                projection.count(count)?;
            }
        }
        Ok(())
    }
}
impl sealed::Source for RetainedRoles {}
impl OwnerGraph for RetainedRoles {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        for slot in self.retained() {
            if let Some(current) = &slot.current {
                current.count(count)?;
            }
            if let Some(retiring) = &slot.retiring {
                retiring.count(count)?;
            }
        }
        for projection in self.construction().into_iter().flatten() {
            projection.count(count)?;
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        for slot in self.retained() {
            if let Some(current) = &slot.current {
                current.cleanup(permit);
            }
            if let Some(retiring) = &slot.retiring {
                retiring.cleanup(permit);
            }
        }
        for projection in self.construction().into_iter().flatten() {
            projection.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut super::Publication<'_>) {
        for slot in self.retained() {
            if let Some(current) = &slot.current {
                current.adopt(publication);
            }
            if let Some(retiring) = &slot.retiring {
                retiring.adopt(publication);
            }
        }
        for projection in self.construction().into_iter().flatten() {
            projection.adopt(publication);
        }
    }
}
/// A real immutable payload and its sole producer registration. Moving this
/// capability transfers the actual root lifetime; cloning raw payloads is absent.
pub struct RetainedRole {
    scope: DocumentRetention,
    kind: RetainedKind,
    owner: Option<RetainedPayload>,
}
impl RetainedRole {
    fn new(scope: &DocumentRetention, kind: RetainedKind) -> Self {
        Self {
            scope: scope.clone(),
            kind,
            owner: None,
        }
    }
    pub fn scope(&self) -> &DocumentRetention {
        &self.scope
    }
    pub(crate) fn is_writer_latest(&self) -> bool {
        self.kind == RetainedKind::WriterLatest
    }
    pub(crate) fn is_writer_active(&self) -> bool {
        self.kind == RetainedKind::WriterActive
    }
    pub(crate) fn is_delivery(&self) -> bool {
        self.kind == RetainedKind::Delivery
    }
    pub(crate) fn is_live(&self) -> bool {
        self.kind == RetainedKind::Live
    }
    pub(crate) fn is_candidate(&self) -> bool {
        self.kind == RetainedKind::Candidate
    }
    pub(crate) fn is_prompt_dispatch(&self) -> bool {
        self.kind == RetainedKind::PromptDispatch
    }
    pub fn snapshot(&self) -> Option<&SnapshotOwner> {
        self.owner.as_ref().and_then(RetainedPayload::snapshot)
    }
    pub(crate) fn dispatch(&self) -> Option<&crate::history::physical::DispatchPayload> {
        self.owner.as_ref().and_then(RetainedPayload::dispatch)
    }
    pub(crate) fn publish_dispatch(
        &mut self,
        dispatch: &mut Self,
        next: SnapshotStaging<'_>,
        request: crate::history::physical::DispatchStaging<'_>,
    ) -> Result<(), RetentionError> {
        if !self.is_live() || !dispatch.is_prompt_dispatch() {
            return Err(RetentionError::WrongPayload);
        }
        if !self.scope.same_document(&dispatch.scope) {
            return Err(RetentionError::ForeignAuthority);
        }
        if dispatch.owner.is_some() {
            return Err(RetentionError::FrozenOwner);
        }
        self.scope
            .replace_with_dispatch(&mut self.owner, &mut dispatch.owner, next, request)
    }
    pub fn replace(&mut self, next: SnapshotStaging<'_>) -> Result<(), RetentionError> {
        self.scope.replace(self.kind, &mut self.owner, next, None)
    }
    pub(crate) fn replace_certified(
        &mut self,
        next: SnapshotStaging<'_>,
        policy: &mut super::PolicyLiability,
    ) -> Result<(), RetentionError> {
        self.scope
            .replace(self.kind, &mut self.owner, next, Some(policy))
    }
    /// A snapshot may escape to another actual role only through this method.
    pub fn clone_from_role(&mut self, source: &Self) -> Result<(), RetentionError> {
        if self.is_prompt_dispatch() {
            return Err(RetentionError::WrongPayload);
        }
        if !self.scope.same_document(&source.scope) {
            return Err(RetentionError::ForeignAuthority);
        }
        match source.snapshot() {
            Some(owner) => self.scope.alias(self.kind, &mut self.owner, owner),
            None => {
                if source.owner.is_some() {
                    return Err(RetentionError::WrongPayload);
                }
                self.clear();
                Ok(())
            }
        }
    }
    pub fn transfer_from(&mut self, source: &mut Self) -> Result<(), RetentionError> {
        self.clone_from_role(source)?;
        source.clear();
        Ok(())
    }
    pub fn clear(&mut self) {
        self.scope.clear(self.kind, &mut self.owner);
    }
}
impl Drop for RetainedRole {
    fn drop(&mut self) {
        self.clear();
    }
}
/// Concrete producer admission. Native I/O/DTO construction never retains the
/// document mutex. Its optional fork source has a genuine real owner token.
pub struct ConstructionRole {
    scope: DocumentRetention,
    kind: ConstructionKind,
    source: Option<SnapshotOwner>,
    phase: Option<super::phase::PhaseOwner>,
}
impl ConstructionRole {
    fn new(scope: &DocumentRetention, kind: ConstructionKind) -> Result<Self, RetentionError> {
        let phase = super::phase::PhaseOwner::new(scope)?;
        scope.publish_phase(kind, phase.identity())?;
        Ok(Self {
            scope: scope.clone(),
            kind,
            source: None,
            phase: Some(phase),
        })
    }
    pub(crate) fn is_writer(&self) -> bool {
        self.kind == ConstructionKind::Writer
    }
    pub(crate) fn is_reader(&self) -> bool {
        self.kind == ConstructionKind::Reader
    }
    pub(crate) fn is_source(&self) -> bool {
        self.kind == ConstructionKind::Source
    }
    pub(crate) fn is_foreground(&self) -> bool {
        self.kind == ConstructionKind::Foreground
    }
    pub(crate) fn construction_key(&self) -> Result<super::ConstructionKey, RetentionError> {
        let phase = self.phase.as_ref().ok_or(RetentionError::FrozenOwner)?;
        if phase.active() {
            Ok(phase.key())
        } else {
            Err(RetentionError::FrozenOwner)
        }
    }
    pub(crate) fn ensure_live_phase(&mut self) -> Result<(), RetentionError> {
        if !self.is_foreground() {
            return Err(RetentionError::WrongPayload);
        }
        if self
            .phase
            .as_ref()
            .is_some_and(super::phase::PhaseOwner::active)
        {
            return Ok(());
        }
        self.begin_phase()
    }
    pub fn close_phase(&mut self) {
        if let Some(phase) = &self.phase {
            phase.close(&self.scope);
        }
        self.clear_source();
    }
    pub fn begin_phase(&mut self) -> Result<(), RetentionError> {
        self.close_phase();
        let next = super::phase::PhaseOwner::new(&self.scope)?;
        self.scope.publish_phase(self.kind, next.identity())?;
        let old = self.phase.replace(next);
        drop(old);
        self.scope.finish_phase_retiring(self.kind);
        Ok(())
    }
    pub fn scope(&self) -> &DocumentRetention {
        &self.scope
    }
    pub fn retain_source(&mut self, source: &RetainedRole) -> Result<(), RetentionError> {
        if !self.scope.same_document(&source.scope) {
            return Err(RetentionError::ForeignAuthority);
        }
        self.install_source(source)
    }
    /// A genuine independent fork deliberately accounts its borrowed source in
    /// the target scope before any target allocation or physical alias escapes.
    pub fn retain_fork_source(&mut self, source: &RetainedRole) -> Result<(), RetentionError> {
        if self.scope.same_document(&source.scope)
            || !self.scope.authority().same(source.scope.authority())
        {
            return Err(RetentionError::ForeignAuthority);
        }
        self.install_source(source)
    }
    fn install_source(&mut self, source: &RetainedRole) -> Result<(), RetentionError> {
        self.construction_key()?.validate_live()?;
        if source.owner.is_some() && source.snapshot().is_none() {
            return Err(RetentionError::WrongPayload);
        }
        self.clear_source();
        if let Some(owner) = source.snapshot() {
            let next = owner.clone_within_role();
            self.scope.source(self.kind, Some(next.projection()))?;
            self.source = Some(next);
        }
        Ok(())
    }
    pub fn clear_source(&mut self) {
        // Physical destruction precedes projection metadata detachment.
        drop(self.source.take());
        // A poisoned account rejects every future admission. Its remaining
        // topology retires when the account itself drops; do not recover it.
        let _ = self.scope.source(self.kind, None);
    }
    pub(crate) fn reserve_pending(
        &self,
        plan: AllocationPlan,
    ) -> Result<PendingAllocation, RetentionError> {
        self.construction_key()?.reserve_pending(plan)
    }
}
impl Drop for ConstructionRole {
    fn drop(&mut self) {
        self.close_phase();
        drop(self.phase.take());
        self.scope.clear_phase(self.kind);
    }
}
/// Minted once; named consuming takes distribute capabilities to actual owners.
pub struct DocumentRoles {
    live: Option<RetainedRole>,
    candidate: Option<RetainedRole>,
    writer_active: Option<RetainedRole>,
    writer_latest: Option<RetainedRole>,
    delivery: Option<RetainedRole>,
    prompt_dispatch: Option<RetainedRole>,
    foreground: Option<ConstructionRole>,
    writer: Option<ConstructionRole>,
    reader: Option<ConstructionRole>,
    source: Option<ConstructionRole>,
}
impl DocumentRoles {
    pub(super) fn new(scope: &DocumentRetention) -> Result<Self, RetentionError> {
        Ok(Self {
            live: Some(RetainedRole::new(scope, RetainedKind::Live)),
            candidate: Some(RetainedRole::new(scope, RetainedKind::Candidate)),
            writer_active: Some(RetainedRole::new(scope, RetainedKind::WriterActive)),
            writer_latest: Some(RetainedRole::new(scope, RetainedKind::WriterLatest)),
            delivery: Some(RetainedRole::new(scope, RetainedKind::Delivery)),
            prompt_dispatch: Some(RetainedRole::new(scope, RetainedKind::PromptDispatch)),
            foreground: Some(ConstructionRole::new(scope, ConstructionKind::Foreground)?),
            writer: Some(ConstructionRole::new(scope, ConstructionKind::Writer)?),
            reader: Some(ConstructionRole::new(scope, ConstructionKind::Reader)?),
            source: Some(ConstructionRole::new(scope, ConstructionKind::Source)?),
        })
    }
    pub fn take_live(&mut self) -> Option<RetainedRole> {
        self.live.take()
    }
    pub fn take_candidate(&mut self) -> Option<RetainedRole> {
        self.candidate.take()
    }
    pub fn take_writer_active(&mut self) -> Option<RetainedRole> {
        self.writer_active.take()
    }
    pub fn take_writer_latest(&mut self) -> Option<RetainedRole> {
        self.writer_latest.take()
    }
    pub fn take_delivery(&mut self) -> Option<RetainedRole> {
        self.delivery.take()
    }
    pub fn take_prompt_dispatch(&mut self) -> Option<RetainedRole> {
        self.prompt_dispatch.take()
    }
    pub fn take_foreground(&mut self) -> Option<ConstructionRole> {
        self.foreground.take()
    }
    pub fn take_writer(&mut self) -> Option<ConstructionRole> {
        self.writer.take()
    }
    pub fn take_source(&mut self) -> Option<ConstructionRole> {
        self.source.take()
    }
    pub fn take_reader(&mut self) -> Option<ConstructionRole> {
        self.reader.take()
    }
}
