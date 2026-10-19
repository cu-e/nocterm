use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

use futures::{StreamExt as _, future::BoxFuture, stream::FuturesUnordered};
use gpui::{App, Entity};
use nocterm_core::persist::ShutdownDeadline;

use crate::{Persisted, StoreError, persisted};

/// Every participant shares this budget, including waiting for active writes.
pub const SHUTDOWN_BUDGET: Duration = Duration::from_millis(180);
type Capture = Box<dyn FnOnce(&mut App) -> BoxFuture<'static, Result<(), StoreError>>>;

/// Explicit application service; it neither registers hooks nor uses globals.
#[derive(Clone, Default)]
pub struct ShutdownCoordinator(Rc<RefCell<Registry>>);

#[derive(Default)]
struct Registry {
    captures: Vec<Capture>,
    closing: bool,
}

/// Failures observed before the shared deadline and unfinished work at expiry.
#[derive(Debug, Default)]
pub struct ShutdownReport {
    failures: Vec<StoreError>,
    timed_out: bool,
}

impl ShutdownReport {
    pub fn failures(&self) -> &[StoreError] {
        &self.failures
    }
    pub fn timed_out(&self) -> bool {
        self.timed_out
    }
}

impl ShutdownCoordinator {
    /// Register an owner's synchronous capture. It must return self-contained work.
    pub fn register_capture(
        &self,
        capture: impl FnOnce(&mut App) -> BoxFuture<'static, Result<(), StoreError>> + 'static,
    ) -> Result<(), StoreError> {
        let mut registry = self.0.borrow_mut();
        if registry.closing {
            return Err(StoreError::Closing);
        }
        registry.captures.push(Box::new(capture));
        Ok(())
    }

    /// Retain the service participant until capture; it owns no view or coordinator.
    pub fn register<T: Clone + PartialEq + Send + 'static>(
        &self,
        entity: &Entity<Persisted<T>>,
    ) -> Result<(), StoreError> {
        let entity = entity.clone();
        self.register_capture(move |cx| persisted::capture(&entity, cx))
    }

    /// Freeze and capture every participant before any background drain begins.
    /// The returned future contains no App, entity or weak entity reads.
    pub fn capture_all(&self, cx: &mut App) -> BoxFuture<'static, ShutdownReport> {
        self.capture_with_budget(cx, SHUTDOWN_BUDGET)
    }

    fn capture_with_budget(
        &self,
        cx: &mut App,
        budget: Duration,
    ) -> BoxFuture<'static, ShutdownReport> {
        let deadline = ShutdownDeadline::new(budget);
        let captures = {
            let mut registry = self.0.borrow_mut();
            registry.closing = true;
            std::mem::take(&mut registry.captures)
        };
        let work = captures
            .into_iter()
            .map(|capture| capture(cx))
            .collect::<Vec<_>>();
        let executor = cx.background_executor().clone();
        Box::pin(async move {
            // Each captured backend receives its own task, so synchronous file
            // work for one participant cannot serialize the others.
            let work = work
                .into_iter()
                .map(|work| executor.spawn(work))
                .collect::<FuturesUnordered<_>>();
            let failures = Arc::new(Mutex::new(Vec::new()));
            let observed = failures.clone();
            let drain = executor.spawn(async move {
                let mut work = work;
                while let Some(result) = work.next().await {
                    if let Err(error) = result {
                        observed
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(error);
                    }
                }
            });
            // Race outside the disk task: a synchronous backend may block its
            // thread, but must never prevent the quit caller's timer from polling.
            let timed_out = matches!(
                futures::future::select(drain, executor.timer(deadline.remaining())).await,
                futures::future::Either::Right(_)
            );
            let mut failures = failures.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if timed_out {
                failures.push(StoreError::DeadlineExceeded);
            }
            ShutdownReport {
                failures,
                timed_out,
            }
        })
    }
}

#[cfg(test)]
mod tests;
