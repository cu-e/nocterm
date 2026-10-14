use super::{
    FactsAdmissionError,
    tickets::{JobKey, job_key},
};
use gpui::BackgroundExecutor;
use std::sync::Arc;
pub const MAX_FACT_JOBS: usize = 32;
#[derive(Clone, Copy)]
enum Turn {
    Probe,
    Flag,
}
pub(super) struct ObservationJobs {
    physical: Vec<JobKey>,
    turn: Turn,
    external: Option<usize>,
    executor: BackgroundExecutor,
}
impl ObservationJobs {
    pub(super) fn new(executor: BackgroundExecutor) -> Self {
        Self {
            physical: Vec::new(),
            turn: Turn::Probe,
            external: None,
            executor,
        }
    }
    pub(super) fn available(&self) -> usize {
        MAX_FACT_JOBS - self.physical.len()
    }
    pub(super) fn executor(&self) -> &BackgroundExecutor {
        &self.executor
    }
    pub(super) fn reserve_probe(
        &mut self,
        needed: usize,
        flags_pending: bool,
    ) -> Result<Vec<JobKey>, FactsAdmissionError> {
        if needed == 0 {
            return Ok(Vec::new());
        }
        let available = self.available();
        if needed > available || (flags_pending && matches!(self.turn, Turn::Flag) && available > 0)
        {
            self.external = Some(needed);
            return Err(FactsAdmissionError::Capacity { needed, available });
        }
        self.external = None;
        self.turn = Turn::Flag;
        Ok(self.reserve(needed))
    }
    pub(super) fn reserve_flag(&mut self) -> Option<JobKey> {
        if self.available() == 0 || (self.external.is_some() && matches!(self.turn, Turn::Probe)) {
            return None;
        }
        self.turn = Turn::Probe;
        self.reserve(1).pop()
    }
    fn reserve(&mut self, needed: usize) -> Vec<JobKey> {
        let keys: Vec<_> = (0..needed).map(|_| job_key()).collect();
        self.physical.extend(keys.iter().cloned());
        keys
    }
    pub(super) fn release(&mut self, key: &JobKey) -> bool {
        let Some(index) = self.physical.iter().position(|job| Arc::ptr_eq(job, key)) else {
            return false;
        };
        self.physical.swap_remove(index);
        true
    }
    pub(super) fn yield_probe_turn(&mut self) {
        self.external = None;
        self.turn = Turn::Flag;
    }
}
