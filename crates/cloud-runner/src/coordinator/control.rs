//! Synchronous revocation and original runtime fencing, independent of disk waits.
use futures::future::AbortHandle;
use nocterm_cloud_model::{
    coordinator::{AuthenticatedCommandSource, CoordinatorError},
    tool::{RunRetirement, RunToolRuntime},
};
use parking_lot::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
pub(super) struct RunControl {
    stopped: AtomicBool,
    task_stopped: AtomicBool,
    abort: Mutex<Option<AbortHandle>>,
    runtime: Mutex<Option<Arc<dyn RunToolRuntime>>>,
    observed: Mutex<Option<nocterm_cloud_model::identity::Timestamp>>,
}
impl RunControl {
    pub(super) fn check(
        &self,
        source: &dyn AuthenticatedCommandSource,
    ) -> Result<(), CoordinatorError> {
        source.check_current()?;
        if self.stopped.load(Ordering::Acquire) {
            return Err(CoordinatorError::Stopped);
        }
        Ok(())
    }
    pub(super) fn check_background(
        &self,
        source: &dyn AuthenticatedCommandSource,
    ) -> Result<(), CoordinatorError> {
        source.check_current()?;
        if self.task_stopped.load(Ordering::Acquire) {
            return Err(CoordinatorError::Stopped);
        }
        Ok(())
    }
    pub(super) fn stop(&self) {
        self.task_stopped.store(true, Ordering::Release);
        self.fence();
    }
    pub(super) fn check_window(
        &self,
        now: nocterm_cloud_model::identity::Timestamp,
        window: nocterm_cloud_model::grant::GrantWindow,
    ) -> Result<(), CoordinatorError> {
        let mut observed = self.observed.lock();
        if observed.is_some_and(|old| now < old) {
            drop(observed);
            self.fence();
            return Err(CoordinatorError::Expired);
        }
        *observed = Some(now);
        if now < window.issued() || now >= window.expires() {
            return Err(CoordinatorError::Expired);
        }
        Ok(())
    }
    pub(super) fn install_abort(&self, abort: AbortHandle) {
        let mut current = self.abort.lock();
        if self.stopped.load(Ordering::Acquire) {
            abort.abort();
        } else {
            *current = Some(abort);
        }
    }
    pub(super) fn install_runtime(
        &self,
        runtime: Arc<dyn RunToolRuntime>,
    ) -> Result<(), CoordinatorError> {
        let mut current = self.runtime.lock();
        if self.stopped.load(Ordering::Acquire) {
            runtime.fence();
            return Err(CoordinatorError::Stopped);
        }
        *current = Some(runtime);
        Ok(())
    }
    pub(super) fn can_resume(&self) -> bool {
        !self.stopped.load(Ordering::Acquire)
    }
    pub(super) fn suspend(&self) {
        if let Some(abort) = self.abort.lock().as_ref() {
            abort.abort();
        }
        if let Some(runtime) = self.runtime.lock().as_ref() {
            runtime.fence();
        }
    }
    pub(super) fn fence(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(abort) = self.abort.lock().as_ref() {
            abort.abort();
        }
        if let Some(runtime) = self.runtime.lock().as_ref() {
            runtime.fence();
        }
    }
    pub(super) async fn retire(&self, reason: RunRetirement) -> Result<(), CoordinatorError> {
        let runtime = self.runtime.lock().clone();
        if let Some(runtime) = runtime {
            // Keep the original runtime reachable by synchronous fencing while
            // its asynchronous retirement is suspended or cancelled.
            let result = runtime.retire(reason).await;
            if result.is_err() {
                runtime.fence();
            }
            let mut current = self.runtime.lock();
            if current
                .as_ref()
                .is_some_and(|held| Arc::ptr_eq(held, &runtime))
            {
                current.take();
            }
            result?;
        }
        Ok(())
    }
}
