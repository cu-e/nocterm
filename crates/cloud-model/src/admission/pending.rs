//! Capacity held before durable acceptance, with no runnable queue entry yet.
use super::{AdmissionError, QueuedRun};
use crate::{authority::RunBinding, identity::SchedulerId};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct QueueReservation {
    owner: SchedulerId,
    token: u64,
    request: QueuedRun,
}
impl QueueReservation {
    pub fn request(&self) -> &QueuedRun {
        &self.request
    }
}

#[derive(Debug, Default)]
pub(super) struct PendingSlots {
    slots: BTreeMap<RunBinding, QueueReservation>,
    next: u64,
}
impl PendingSlots {
    pub(super) fn insert(
        &mut self,
        owner: SchedulerId,
        request: QueuedRun,
    ) -> Result<QueueReservation, AdmissionError> {
        let token = self.next.checked_add(1).ok_or(AdmissionError::Exhausted)?;
        let reservation = QueueReservation {
            owner,
            token,
            request,
        };
        self.slots
            .insert(reservation.request.binding().clone(), reservation.clone());
        self.next = token;
        Ok(reservation)
    }
    pub(super) fn take(
        &mut self,
        owner: SchedulerId,
        reservation: &QueueReservation,
    ) -> Result<QueuedRun, AdmissionError> {
        let original = self
            .slots
            .get(reservation.request.binding())
            .ok_or(AdmissionError::StaleLease)?;
        if reservation.owner != owner
            || original.token != reservation.token
            || original.request != reservation.request
        {
            return Err(AdmissionError::StaleLease);
        }
        self.slots
            .remove(reservation.request.binding())
            .map(|slot| slot.request)
            .ok_or(AdmissionError::StaleLease)
    }
    pub(super) fn get(&self, run: &RunBinding) -> Option<&QueuedRun> {
        self.slots.get(run).map(|slot| &slot.request)
    }
    pub(super) fn cancel(&mut self, run: &RunBinding) -> Option<QueuedRun> {
        self.slots.remove(run).map(|slot| slot.request)
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = &QueuedRun> {
        self.slots.values().map(|slot| &slot.request)
    }
    pub(super) fn len(&self) -> usize {
        self.slots.len()
    }
}
