//! A chat's share of the Local installation's history admission: one payload slot and the
//! bytes of its transcript, reserved before the transcript grows.
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
use nocterm_core::allocation::AllocationLease;
use std::rc::Rc;

pub(crate) struct Residency {
    admission: HistoryAdmissionHandle,
    registration: Option<HistoryRegistration>,
    payload: Option<HistoryPayloadPermit>,
    bytes: Option<AllocationLease>,
    shown: bool,
}
struct Usage(WeakEntity<CloudChat>);
impl HistoryParticipant for Usage {
    fn entity_id(&self) -> Option<EntityId> {
        Some(self.0.entity_id())
    }
    fn usage(&self, cx: &App) -> Result<Option<HistoryUsage>, AgentStorageError> {
        Ok(self.0.upgrade().map(|chat| {
            let chat = chat.read(cx);
            HistoryUsage::new(0, chat.updated_at(), chat.is_shown())
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
impl Residency {
    pub(crate) fn new(admission: HistoryAdmissionHandle, payload: HistoryPayloadPermit) -> Self {
        Self {
            admission,
            registration: None,
            payload: Some(payload),
            bytes: None,
            shown: true,
        }
    }
    pub(crate) fn register(
        &mut self,
        cx: &Context<CloudChat>,
    ) -> Result<(), HistoryAdmissionError> {
        self.registration = Some(self.admission.register(Rc::new(Usage(cx.weak_entity())))?);
        Ok(())
    }
    /// Reserves `bytes` for the transcript in place of the earlier reservation, and the
    /// payload slot when the chat was evicted.
    pub(crate) fn fit(
        &mut self,
        bytes: usize,
        cx: &mut Context<CloudChat>,
    ) -> Result<(), HistoryAdmissionError> {
        if self
            .bytes
            .as_ref()
            .is_some_and(|held| held.bytes() >= bytes)
        {
            return Ok(());
        }
        let payload = match self.payload.take() {
            Some(payload) => payload,
            None => self
                .admission
                .reserve_payload_reclaiming(Some(cx.entity_id()), cx)?,
        };
        let reserved = self.admission.reserve_bytes(
            self.registration
                .as_ref()
                .ok_or(HistoryAdmissionError::Closed)?,
            bytes.saturating_mul(2),
            cx,
        );
        self.payload = Some(payload);
        let reserved = reserved?;
        self.bytes = Some(AllocationLease::new(reserved.bytes(), 0, reserved));
        Ok(())
    }
    pub(crate) fn shown(&self) -> bool {
        self.shown
    }
    pub(crate) fn set_shown(&mut self, shown: bool) {
        self.shown = shown;
    }
    pub(crate) fn evicted(&mut self) {
        self.payload.take();
        self.bytes.take();
    }
}
