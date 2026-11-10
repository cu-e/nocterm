//! The original executor owns a finite native command budget through actual cleanup.
use nocterm_session::ExecError;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const MAX_COMMANDS: usize = 32;
#[derive(Debug, Default)]
pub(super) struct Admission {
    active: AtomicUsize,
}
impl Admission {
    pub(super) fn reserve(self: &Arc<Self>) -> Result<Arc<Permit>, ExecError> {
        self.active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_COMMANDS).then_some(active + 1)
            })
            .map_err(|_| {
                ExecError::Failed(
                    "Local command capacity is full (32). Wait for a command to finish.".into(),
                )
            })?;
        Ok(Arc::new(Permit(self.clone())))
    }
}
pub(super) struct Permit(Arc<Admission>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::AcqRel);
    }
}
