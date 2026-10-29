//! Inactivity belongs to the exact native account whose work completed.
use crate::account::source::CapturedAccount;
use std::{
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};
use tokio::{runtime::Handle, task::AbortHandle};

#[derive(Default)]
struct IdleState {
    limit: Option<Duration>,
    timer: Option<AbortHandle>,
}

pub(crate) struct Idle {
    handle: Handle,
    state: Mutex<IdleState>,
}

impl Idle {
    pub(crate) fn new(handle: Handle) -> Self {
        Self {
            handle,
            state: Mutex::default(),
        }
    }
    fn state(&self) -> std::sync::MutexGuard<'_, IdleState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
    pub(crate) fn set(&self, duration: Duration, original: Option<Arc<CapturedAccount>>) {
        self.state().limit = Some(duration);
        if let Some(original) = original {
            self.touch(&original);
        }
    }
    pub(crate) fn touch(&self, original: &Arc<CapturedAccount>) {
        // Pool membership stays admitted while the timer state changes. Late A
        // cannot abort or restart B's countdown.
        let _ = original.publish(|| {
            let mut state = self.state();
            if let Some(timer) = state.timer.take() {
                timer.abort();
            }
            let Some(limit) = state.limit else {
                return;
            };
            let owner = original.clone();
            state.timer = Some(
                self.handle
                    .spawn(async move {
                        tokio::time::sleep(limit).await;
                        let _ = owner.with_current(|keys| {
                            owner.slot()?.lock_keys_if(keys);
                            Ok(())
                        });
                    })
                    .abort_handle(),
            );
        });
    }
    /// Caller holds original native membership admission for explicit lock.
    pub(crate) fn stop(&self) {
        if let Some(timer) = self.state().timer.take() {
            timer.abort();
        }
    }
}

impl Drop for Idle {
    fn drop(&mut self) {
        if let Some(timer) = self
            .state
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .timer
            .take()
        {
            timer.abort();
        }
    }
}
