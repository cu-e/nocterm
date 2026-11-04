//! Weak projection participation in the retained Local installation admission.
use crate::CloudChat;
use gpui::{App, Context, EntityId, WeakEntity};
use nocterm_agent_runtime::{
    HistoryUsage,
    history_admission::{
        HistoryAdmissionError, HistoryAdmissionHandle, HistoryParticipant, HistoryPayloadPermit,
        HistoryRegistration,
    },
};
use nocterm_ai::storage::AgentStorageError;
use nocterm_cloud_model::projection::ports::AllocationRequirements;
use nocterm_core::allocation::AllocationLease;
use std::{rc::Rc, sync::Arc};

pub(crate) struct ProjectionResidency {
    admission: HistoryAdmissionHandle,
    registration: Option<HistoryRegistration>,
    payload: Option<HistoryPayloadPermit>,
    shown: bool,
}
struct ProjectionUsage(WeakEntity<CloudChat>);
impl HistoryParticipant for ProjectionUsage {
    fn entity_id(&self) -> Option<EntityId> {
        Some(self.0.entity_id())
    }
    fn usage(&self, cx: &App) -> Result<Option<HistoryUsage>, AgentStorageError> {
        Ok(self.0.upgrade().map(|chat| {
            let chat = chat.read(cx);
            HistoryUsage::new(0, chat.projection().updated_at(), chat.is_shown())
        }))
    }
    fn evict(&self, cx: &mut App) -> bool {
        self.0
            .upgrade()
            .is_some_and(|chat| chat.update(cx, |chat, cx| chat.evict(cx)))
    }
    fn oldest_payload(&self, cx: &App) -> Option<u64> {
        let chat = self.0.upgrade()?;
        let chat = chat.read(cx);
        chat.can_evict()
            .then(|| {
                chat.residency
                    .payload
                    .as_ref()
                    .map(HistoryPayloadPermit::order)
            })
            .flatten()
    }
    fn evict_payload(&self, cx: &mut App) -> bool {
        self.evict(cx)
    }
}
impl ProjectionResidency {
    pub(crate) fn new(admission: HistoryAdmissionHandle, payload: HistoryPayloadPermit) -> Self {
        Self {
            admission,
            registration: None,
            payload: Some(payload),
            shown: true,
        }
    }
    pub(crate) fn register(
        &mut self,
        cx: &Context<CloudChat>,
    ) -> Result<(), HistoryAdmissionError> {
        self.registration = Some(
            self.admission
                .register(Rc::new(ProjectionUsage(cx.weak_entity())))?,
        );
        Ok(())
    }
    /// A new independently retained generation reserves both installation bounds
    /// before its data is materialized. Cloned leases share these exact proofs.
    pub(crate) fn allocate(
        &self,
        bytes: usize,
        cx: &mut Context<CloudChat>,
    ) -> Result<(AllocationLease, HistoryPayloadPermit), HistoryAdmissionError> {
        let payload = self
            .admission
            .reserve_payload_reclaiming(Some(cx.entity_id()), cx)?;
        let bytes = self.admission.reserve_bytes(
            self.registration
                .as_ref()
                .ok_or(HistoryAdmissionError::Closed)?,
            bytes,
            cx,
        )?;
        let lease = AllocationLease::new(bytes.bytes(), 1, Arc::new((bytes, payload.clone())));
        Ok((lease, payload))
    }
    /// Bytes without a payload permit, for presentation text kept beside the projection.
    pub(crate) fn retained_text(
        &self,
        bytes: usize,
        cx: &mut Context<CloudChat>,
    ) -> Result<AllocationLease, HistoryAdmissionError> {
        let bytes = self.admission.reserve_bytes(
            self.registration
                .as_ref()
                .ok_or(HistoryAdmissionError::Closed)?,
            bytes,
            cx,
        )?;
        Ok(AllocationLease::new(bytes.bytes(), 0, bytes))
    }
    pub(crate) fn work(
        &self,
        required: AllocationRequirements,
        cx: &mut Context<CloudChat>,
    ) -> Result<AllocationLease, HistoryAdmissionError> {
        if required.payloads() > nocterm_agent_runtime::history_admission::MAX_HISTORY_PAYLOADS {
            return Err(HistoryAdmissionError::PayloadCapacity);
        }
        let mut payloads = Vec::with_capacity(required.payloads());
        for _ in 0..required.payloads() {
            payloads.push(
                self.admission
                    .reserve_payload_reclaiming(Some(cx.entity_id()), cx)?,
            );
        }
        let bytes = self.admission.reserve_bytes(
            self.registration
                .as_ref()
                .ok_or(HistoryAdmissionError::Closed)?,
            required.bytes(),
            cx,
        )?;
        Ok(AllocationLease::new(
            bytes.bytes(),
            payloads.len(),
            (bytes, payloads),
        ))
    }
    pub(crate) fn urgent(
        &self,
        required: AllocationRequirements,
        cx: &mut Context<CloudChat>,
    ) -> Result<AllocationLease, HistoryAdmissionError> {
        self.admission.reserve_urgent_control(
            self.registration
                .as_ref()
                .ok_or(HistoryAdmissionError::Closed)?,
            required.bytes(),
            required.payloads(),
            cx,
        )
    }
    pub(crate) fn published(&mut self, payload: HistoryPayloadPermit) {
        self.payload = Some(payload);
    }
    pub(crate) fn shown(&self) -> bool {
        self.shown
    }
    pub(crate) fn set_shown(&mut self, shown: bool) {
        self.shown = shown;
    }
    pub(crate) fn evicted(&mut self) {
        self.payload.take();
    }
}
