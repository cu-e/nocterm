//! Prepared local bodies remain accounted before any chat entity owns them.
use crate::{
    HistoryUsage,
    history_admission::{
        HistoryAdmissionHandle, HistoryParticipant, HistoryPayloadPermit, HistoryRegistration,
    },
};
use gpui::App;
use nocterm_ai::{
    execution_origin::ChatKey,
    history::{ChatSummary, SavedChat},
    storage::AgentStorageError,
};
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

pub(super) struct PreparedRecords {
    records: Rc<RefCell<PreparedBatch>>,
    registration: HistoryRegistration,
    allocations: crate::history_admission::LocalHistoryAllocations,
}
struct PreparedBatch {
    values: Option<Vec<SavedChat>>,
    allocation: Option<HistoryPayloadPermit>,
}
struct PreparedUsage(Weak<RefCell<PreparedBatch>>);
impl HistoryParticipant for PreparedUsage {
    fn usage(&self, _cx: &App) -> Result<Option<HistoryUsage>, AgentStorageError> {
        Ok(self.0.upgrade().map(|records| {
            HistoryUsage::new(
                records.borrow().values.as_ref().map_or(0, |values| {
                    values.iter().map(SavedChat::resident_bytes).sum()
                }),
                0,
                true,
            )
        }))
    }
}
impl PreparedRecords {
    pub(super) fn prepared_count(&self) -> Option<usize> {
        self.records.borrow().values.as_ref().map(Vec::len)
    }
    pub(super) fn new(
        records: Option<Vec<SavedChat>>,
        admission: &HistoryAdmissionHandle,
        allocation: Option<HistoryPayloadPermit>,
        runtime: gpui::EntityId,
    ) -> Self {
        let records = Rc::new(RefCell::new(PreparedBatch {
            values: records,
            allocation,
        }));
        let registration = admission
            .register(Rc::new(PreparedUsage(Rc::downgrade(&records))))
            .expect("a new installation admission is open");
        Self {
            records,
            registration,
            allocations: crate::history_admission::LocalHistoryAllocations::new(
                admission.clone(),
                runtime,
            ),
        }
    }
    pub(super) fn rows(&self) -> Option<Vec<ChatSummary>> {
        self.records
            .borrow()
            .values
            .as_ref()
            .map(|records| records.iter().map(ChatSummary::from_chat).collect())
    }
    pub(super) fn take(
        &self,
        key: &ChatKey,
        payload: HistoryPayloadPermit,
        cx: &mut App,
    ) -> Result<Option<nocterm_core::allocation::Allocated<SavedChat>>, AgentStorageError> {
        let values = self.records.borrow();
        let Some(records) = &values.values else {
            return Ok(None);
        };
        let Some(index) = records.iter().position(|chat| chat.header.id == key.id()) else {
            return Ok(None);
        };
        let bytes = records[index].resident_bytes();
        let remaining = records
            .iter()
            .enumerate()
            .filter(|(position, _)| *position != index)
            .fold(0usize, |total, (_, chat)| {
                total.saturating_add(chat.resident_bytes())
            });
        drop(values);
        // Admit before removing the source body. No callbacks/await occur between
        // reserving this immutable destination and moving that exact owned body.
        let allocation = self.allocations.transfer_prepared(
            &self.registration,
            remaining,
            bytes,
            payload,
            cx,
        )?;
        let mut batch = self.records.borrow_mut();
        let Some(records) = batch.values.as_mut() else {
            return Err(AgentStorageError::Interrupted);
        };
        let Some(index) = records.iter().position(|chat| chat.header.id == key.id()) else {
            return Err(AgentStorageError::Interrupted);
        };
        let chat = records.remove(index);
        if records.is_empty() {
            batch.allocation.take();
        }
        Ok(Some(nocterm_core::allocation::Allocated::new(
            chat, allocation,
        )))
    }
    pub(super) fn discard(&self, key: &ChatKey) {
        let mut batch = self.records.borrow_mut();
        if let Some(records) = &mut batch.values {
            if let Some(index) = records.iter().position(|chat| chat.header.id == key.id()) {
                drop(records.remove(index));
            }
            if records.is_empty() {
                batch.allocation.take();
            }
        }
    }
}
