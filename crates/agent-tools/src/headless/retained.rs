//! Chat-scoped physical lifetimes and job budgets, independent of foreground views.
use super::{HeadlessError, SessionRegistry, commands::BackgroundJobs, physical::PhysicalTerminal};
use nocterm_cloud_model::{
    authority::{RunBinding, SessionReceipt, StreamBinding},
    identity::RemoteSessionId,
    tool::{BackgroundCommandAuthority, BackgroundCommandCleanup},
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, PoisonError},
};

struct PhysicalState {
    entries: BTreeMap<RemoteSessionId, Arc<Mutex<PhysicalTerminal>>>,
    closed: bool,
}
pub(super) struct RetainedOwner {
    stream: StreamBinding,
    state: Mutex<PhysicalState>,
    jobs: Arc<Mutex<BackgroundJobs>>,
    observations: Arc<dyn BackgroundCommandAuthority>,
    cleanup: Arc<dyn BackgroundCommandCleanup>,
}
impl RetainedOwner {
    pub(super) fn insert(&self, entry: Arc<Mutex<PhysicalTerminal>>) -> Result<(), HeadlessError> {
        let id = entry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .receipt()
            .id();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.closed {
            return Err(HeadlessError::Closed);
        }
        if state.entries.contains_key(&id) {
            return Err(HeadlessError::IdentityConflict);
        }
        if state.entries.len() >= 16 {
            return Err(HeadlessError::Capacity);
        }
        state.entries.insert(id, entry);
        Ok(())
    }
    pub(super) fn get(
        &self,
        receipt: &SessionReceipt,
    ) -> Result<Arc<Mutex<PhysicalTerminal>>, HeadlessError> {
        self.original(receipt, false)
    }
    pub(super) fn reattach(
        &self,
        receipt: &SessionReceipt,
    ) -> Result<Arc<Mutex<PhysicalTerminal>>, HeadlessError> {
        self.original(receipt, true)
    }
    fn original(
        &self,
        receipt: &SessionReceipt,
        publish: bool,
    ) -> Result<Arc<Mutex<PhysicalTerminal>>, HeadlessError> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.closed {
            return Err(HeadlessError::Closed);
        }
        let entry = state
            .entries
            .get(&receipt.id())
            .ok_or(HeadlessError::StaleSession)?;
        {
            let mut physical = entry.lock().unwrap_or_else(PoisonError::into_inner);
            if physical.receipt() != receipt {
                return Err(HeadlessError::StaleSession);
            }
            if publish {
                physical.publish()?;
            }
        }
        Ok(entry.clone())
    }
    pub(super) fn publish_initial(
        &self,
        original: &Arc<Mutex<PhysicalTerminal>>,
        owner: Arc<()>,
    ) -> Result<(), HeadlessError> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.closed {
            return Err(HeadlessError::Closed);
        }
        let mut physical = original.lock().unwrap_or_else(PoisonError::into_inner);
        if !state
            .entries
            .get(&physical.receipt().id())
            .is_some_and(|entry| Arc::ptr_eq(entry, original))
        {
            return Err(HeadlessError::StaleSession);
        }
        physical.publish_initial_owned(owner)
    }
    pub(super) fn discard_pending(&self, original: &Arc<Mutex<PhysicalTerminal>>) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut physical = original.lock().unwrap_or_else(PoisonError::into_inner);
        let id = physical.receipt().id();
        if state
            .entries
            .get(&id)
            .is_some_and(|entry| Arc::ptr_eq(entry, original))
            && physical.retire_pending()
        {
            state.entries.remove(&id);
        }
    }
    pub(super) fn remove_if_original(&self, original: &Arc<Mutex<PhysicalTerminal>>) {
        let id = original
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .receipt()
            .id();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state
            .entries
            .get(&id)
            .is_some_and(|entry| Arc::ptr_eq(entry, original))
        {
            state.entries.remove(&id);
        }
    }
    pub(super) fn jobs(&self) -> Arc<Mutex<BackgroundJobs>> {
        self.jobs.clone()
    }
    pub(super) fn observations(&self) -> Arc<dyn BackgroundCommandAuthority> {
        self.observations.clone()
    }
    pub(super) fn cleanup(&self) -> Arc<dyn BackgroundCommandCleanup> {
        self.cleanup.clone()
    }
    fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.closed = true;
        for entry in state.entries.values() {
            entry
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retire();
        }
        self.jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .cancel_all();
    }
}
impl Drop for RetainedOwner {
    fn drop(&mut self) {
        self.close();
    }
}

/// Explicit supervisor-owned lifetime for one account/chat/runner generation.
/// Dropping a view preserves physical state; dropping the last owner closes it.
/// This descriptive binding is not a grant: every new run needs its own authority.
#[derive(Clone)]
pub struct RetainedTerminals(Arc<RetainedOwner>);
impl RetainedTerminals {
    /// Reclaims only the actual generation whose sole reader reported Lost.
    /// Old observations cannot remove a replacement with the same receipt.
    pub fn retire_observed(&self, observation: &super::initial::SessionObservation) -> bool {
        let removed = {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            let id = observation.receipt().id();
            let matches = state.entries.get(&id).is_some_and(|entry| {
                entry
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .observed_loss(observation)
            });
            if matches {
                state.entries.remove(&id)
            } else {
                None
            }
        };
        if let Some(entry) = removed {
            // Native retirement/drop never runs under the retained-owner lock.
            entry
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retire();
            true
        } else {
            false
        }
    }
    pub fn new(
        run: &RunBinding,
        observations: Arc<dyn BackgroundCommandAuthority>,
        cleanup: Arc<dyn BackgroundCommandCleanup>,
    ) -> Self {
        Self(Arc::new(RetainedOwner {
            stream: run.stream(),
            state: Mutex::new(PhysicalState {
                entries: BTreeMap::new(),
                closed: false,
            }),
            jobs: Arc::new(Mutex::new(BackgroundJobs::new())),
            observations,
            cleanup,
        }))
    }
    pub fn view(&self, run: RunBinding) -> Result<SessionRegistry, HeadlessError> {
        if run.stream() != self.0.stream {
            return Err(nocterm_cloud_model::authority::AuthorityError::WrongScope.into());
        }
        if self
            .0
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed
        {
            return Err(HeadlessError::Closed);
        }
        Ok(SessionRegistry::retained(run, Arc::downgrade(&self.0)))
    }
    /// Explicit physical retirement. Other views of this receipt become stale.
    pub fn remove(&self, receipt: &SessionReceipt) -> Result<(), HeadlessError> {
        let original = self.0.get(receipt)?;
        let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state
            .entries
            .get(&receipt.id())
            .is_some_and(|entry| Arc::ptr_eq(entry, &original))
        {
            state.entries.remove(&receipt.id());
            original
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retire();
            return Ok(());
        }
        Err(HeadlessError::StaleSession)
    }
    /// Revoke admission and stop all physical connections/jobs. It never claims
    /// that cancelling a native observer reverses remote descendants or effects.
    pub fn close(&self) {
        self.0.close();
    }
}
