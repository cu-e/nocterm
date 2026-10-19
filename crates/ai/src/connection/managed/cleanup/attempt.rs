//! Attempt ownership separates a dormant driver from its shared result, avoiding cycles.
use super::{header, waiters::Waiters, *};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
struct ResultBody {
    state: Mutex<ResultState>,
    _allocation: NativeAllocation,
}
struct ResultState {
    result: Option<ManagedShutdownResult>,
    waiters: Waiters,
}
struct AttemptBody {
    driver: Mutex<Option<ManagedSourceFuture<()>>>,
    result: Shared<ResultBody>,
    _allocation: NativeAllocation,
    _lifetime: SourceLifetime,
}
#[derive(Clone)]
pub(super) struct ShutdownAttempt(Shared<AttemptBody>);
impl ShutdownAttempt {
    pub(super) fn prepare(
        commands: ManagedCommandsHandle,
        context: &SourceContext,
        lifetime: SourceLifetime,
        final_intent: bool,
    ) -> Result<Self, NativeAllocationError> {
        let attempt_allocation = header::<AttemptBody>(context)?;
        let result_allocation = header::<ResultBody>(context)?;
        let failure = commands.prepare_cleanup_failure()?;
        let result = Shared::new(ResultBody {
            state: Mutex::new(ResultState {
                result: None,
                waiters: Waiters::new(),
            }),
            _allocation: result_allocation,
        });
        let publisher = result.clone();
        let driver = ManagedSourceFuture::new(lifetime.clone(), async move {
            // Native invocation belongs to this first poll, preserving lazy
            // construction and cancellation of a never-started attempt.
            let operation = if final_intent {
                commands.shutdown_finally()
            } else {
                commands.shutdown_gracefully()
            };
            let outcome = match operation {
                Ok(work) => work.await,
                Err(error) => Err(ManagedCommandError::Admission(error)),
            };
            let outcome = outcome.map_err(|error| failure.capture(error));
            let waiters = {
                let mut state = publisher
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                state.result = Some(outcome);
                std::mem::replace(&mut state.waiters, Waiters::new())
            };
            waiters.wake();
        })?;
        Ok(Self(Shared::new(AttemptBody {
            driver: Mutex::new(Some(driver)),
            result,
            _allocation: attempt_allocation,
            _lifetime: lifetime,
        })))
    }
    pub(super) fn failed(&self) -> bool {
        self.0
            .result
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .result
            .as_ref()
            .is_some_and(Result::is_err)
    }
    pub(super) fn join(&self) -> Result<ManagedShutdownWaiter, NativeAllocationError> {
        let mut state = self
            .0
            .result
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let index = if state.result.is_some() {
            None
        } else {
            Some(state.waiters.register(self.0._allocation.source())?)
        };
        Ok(ManagedShutdownWaiter {
            attempt: self.clone(),
            index,
        })
    }
}
/// One registered observation of the original attempt. Dropping it unregisters
/// only that waiter and never cancels an already launched native operation.
pub struct ManagedShutdownWaiter {
    attempt: ShutdownAttempt,
    index: Option<usize>,
}
impl ManagedShutdownWaiter {
    /// The service schedules this already-paid work on its existing executor.
    /// Exactly one first poll can take the driver; no task is started by joining.
    pub fn take_driver(&mut self) -> Option<ManagedSourceFuture<()>> {
        self.attempt
            .0
            .driver
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
    }
    pub fn result(&self) -> Option<ManagedShutdownResult> {
        self.attempt
            .0
            .result
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .result
            .clone()
    }
}
impl Future for ManagedShutdownWaiter {
    type Output = ManagedShutdownResult;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut state = this
            .attempt
            .0
            .result
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(result) = &state.result {
            return Poll::Ready(result.clone());
        }
        if let Some(index) = this.index {
            state.waiters.update(index, cx.waker());
        }
        Poll::Pending
    }
}
impl Drop for ManagedShutdownWaiter {
    fn drop(&mut self) {
        if let Some(index) = self.index {
            self.attempt
                .0
                .result
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .waiters
                .remove(index);
        }
    }
}
