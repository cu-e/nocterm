//! Shared pure scheduler with reservations that survive only to durable admission.
use nocterm_cloud_model::{
    admission::*, authority::RunBinding, coordinator::CoordinatorError, identity::*,
};
use parking_lot::Mutex;
use std::{collections::BTreeMap, sync::Arc};

pub(super) struct RunQueue {
    scheduler: Mutex<FairAdmission>,
    pending: Mutex<BTreeMap<RunBinding, QueueReservation>>,
}
impl RunQueue {
    pub(super) fn new(id: SchedulerId, limits: AdmissionLimits) -> Self {
        Self {
            scheduler: Mutex::new(FairAdmission::new(id, limits)),
            pending: Mutex::new(BTreeMap::new()),
        }
    }
    pub(super) fn prepare(
        self: &Arc<Self>,
        request: QueuedRun,
        now: Timestamp,
    ) -> Result<PendingRun, CoordinatorError> {
        let binding = request.binding().clone();
        let reservation = self
            .scheduler
            .lock()
            .prepare(request, now)
            .map_err(|_| CoordinatorError::Overloaded)?;
        Ok(PendingRun {
            queue: self.clone(),
            binding,
            reservation: Some(reservation),
        })
    }
    pub(super) fn activate(
        &self,
        binding: &RunBinding,
        now: Timestamp,
    ) -> Result<(), CoordinatorError> {
        let Some(reservation) = self.pending.lock().remove(binding) else {
            return Ok(());
        };
        self.scheduler
            .lock()
            .activate(&reservation, now)
            .map_err(|_| CoordinatorError::Expired)
    }
    pub(super) fn reserve(&self, now: Timestamp) -> Result<AdmissionDecision, CoordinatorError> {
        self.scheduler
            .lock()
            .reserve(now)
            .map_err(|_| CoordinatorError::Stopped)
    }
    pub(super) fn release(&self, lease: &AdmissionLease) -> Result<(), CoordinatorError> {
        self.scheduler
            .lock()
            .release(lease)
            .map_err(|_| CoordinatorError::Conflict)
    }
    pub(super) fn cancel(&self, binding: &RunBinding) {
        self.pending.lock().remove(binding);
        self.scheduler.lock().cancel_queued(binding);
    }
    pub(super) fn expired_active(&self, now: Timestamp) -> Vec<AdmissionLease> {
        self.scheduler.lock().expired_active(now)
    }
}
pub(super) struct PendingRun {
    queue: Arc<RunQueue>,
    binding: RunBinding,
    reservation: Option<QueueReservation>,
}
impl PendingRun {
    pub(super) fn admitted(mut self) {
        if let Some(reservation) = self.reservation.take() {
            self.queue
                .pending
                .lock()
                .insert(self.binding.clone(), reservation);
        }
    }
}
impl Drop for PendingRun {
    fn drop(&mut self) {
        if let Some(reservation) = self.reservation.take() {
            let _ = self.queue.scheduler.lock().abandon(&reservation);
        }
    }
}
