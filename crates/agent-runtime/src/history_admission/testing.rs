//! Shared external history participant for Local/remote admission behavior tests.
use super::{HistoryParticipant, HistoryPayloadPermit};
use crate::HistoryUsage;
use gpui::App;
use nocterm_ai::storage::AgentStorageError;
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

#[derive(Default)]
struct ProbeState {
    usage: Option<HistoryUsage>,
    evictable: bool,
    payloads: Vec<HistoryPayloadPermit>,
    evictions: usize,
    on_evict: Option<Rc<dyn Fn(&mut App)>>,
    false_payload_success: bool,
}
/// The participant adapter keeps only a weak reference to this external owner.
#[derive(Clone, Default)]
pub struct HistoryProbe(Rc<RefCell<ProbeState>>);
struct ProbeParticipant(Weak<RefCell<ProbeState>>);
impl HistoryProbe {
    /// Uses the real installation admission owner without a connector fixture.
    pub fn installation() -> super::HistoryAdmissionHandle {
        super::HistoryAdmissionHandle::new()
    }
    /// Drives the real allocation transfer through the shared test collaborator.
    pub fn transfer_buffer(
        admission: &super::HistoryAdmissionHandle,
        allocation: super::HistoryBytesPermit,
        bytes: usize,
    ) -> Result<super::HistoryBytesPermit, super::HistoryAdmissionError> {
        admission.shrink_buffer(allocation, bytes)
    }
    /// Exercise a dishonest/no-progress collaborator without a second fake.
    pub fn report_payload_success_without_release(&self) {
        self.0.borrow_mut().false_payload_success = true;
    }
    pub fn participant(&self) -> Rc<dyn HistoryParticipant> {
        Rc::new(ProbeParticipant(Rc::downgrade(&self.0)))
    }
    pub fn set_history(&self, usage: HistoryUsage, evictable: bool) {
        let mut state = self.0.borrow_mut();
        state.usage = Some(usage);
        state.evictable = evictable;
    }
    pub fn retain_payload(&self, permit: HistoryPayloadPermit) {
        self.0.borrow_mut().payloads.push(permit);
    }
    pub fn payload_count(&self) -> usize {
        self.0.borrow().payloads.len()
    }
    pub fn history(&self) -> Option<HistoryUsage> {
        self.0.borrow().usage
    }
    pub fn evictions(&self) -> usize {
        self.0.borrow().evictions
    }
    pub fn on_evict(&self, callback: Rc<dyn Fn(&mut App)>) {
        self.0.borrow_mut().on_evict = Some(callback);
    }
}
impl HistoryParticipant for ProbeParticipant {
    fn usage(&self, _cx: &App) -> Result<Option<HistoryUsage>, AgentStorageError> {
        Ok(self.0.upgrade().and_then(|state| state.borrow().usage))
    }
    fn evict(&self, cx: &mut App) -> bool {
        let Some(owner) = self.0.upgrade() else {
            return false;
        };
        let callback = {
            let mut state = owner.borrow_mut();
            if !state.evictable || state.usage.is_some_and(|usage| usage.shown()) {
                return false;
            }
            state.usage = None;
            state.evictions += 1;
            state.on_evict.clone()
        };
        if let Some(callback) = callback {
            callback(cx);
        }
        true
    }
    fn oldest_payload(&self, _cx: &App) -> Option<u64> {
        self.0.upgrade().and_then(|owner| {
            let state = owner.borrow();
            if !state.evictable || state.usage.is_some_and(|usage| usage.shown()) {
                return None;
            }
            state.payloads.iter().map(HistoryPayloadPermit::order).min()
        })
    }
    fn evict_payload(&self, _cx: &mut App) -> bool {
        let Some(owner) = self.0.upgrade() else {
            return false;
        };
        let mut state = owner.borrow_mut();
        if !state.evictable || state.usage.is_some_and(|usage| usage.shown()) {
            return false;
        }
        state.evictions += 1;
        if state.false_payload_success {
            return true;
        }
        let Some(index) = state
            .payloads
            .iter()
            .enumerate()
            .min_by_key(|(_, permit)| permit.order())
            .map(|(index, _)| index)
        else {
            return false;
        };
        state.payloads.remove(index);
        true
    }
}
