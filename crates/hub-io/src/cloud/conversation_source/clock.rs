//! One permanently fenced original-source clock, independent of network progress.
use nocterm_cloud_model::{identity::Timestamp, projection::ProjectionError, tool::ToolTimer};
use std::sync::{Arc, Mutex};

pub(super) struct SourceClock {
    timer: Arc<dyn ToolTimer>,
    observed: Mutex<ClockState>,
}
struct ClockState {
    last: Option<Timestamp>,
    failed: bool,
}
impl SourceClock {
    pub(super) fn new(timer: Arc<dyn ToolTimer>) -> Self {
        Self {
            timer,
            observed: Mutex::new(ClockState {
                last: None,
                failed: false,
            }),
        }
    }
    pub(super) fn now(&self) -> Result<Timestamp, ProjectionError> {
        let mut state = self
            .observed
            .lock()
            .map_err(|_| ProjectionError::Unavailable)?;
        let now = self.timer.now();
        if now.as_millis() == 0 || state.failed || state.last.is_some_and(|last| now < last) {
            state.failed = true;
            return Err(ProjectionError::Rollback);
        }
        state.last = Some(now);
        Ok(now)
    }
    pub(super) fn millis(&self) -> Result<i64, ProjectionError> {
        i64::try_from(self.now()?.as_millis()).map_err(|_| ProjectionError::Rollback)
    }
    pub(super) fn timer(&self) -> &Arc<dyn ToolTimer> {
        &self.timer
    }
}
