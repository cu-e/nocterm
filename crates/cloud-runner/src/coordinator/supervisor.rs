//! Tracked work outlives a network waiter, with reserved retirement capacity.
use futures::{FutureExt, channel::oneshot, future::BoxFuture};
use nocterm_cloud_model::{coordinator::*, tool::*};
use nocterm_core::persist::ShutdownDeadline;
use parking_lot::Mutex;
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::{runtime::Handle, sync::Notify};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TaskClass {
    Command,
    Urgent,
    Worker,
    Retirement,
    Native,
}
impl TaskClass {
    fn index(self) -> usize {
        match self {
            Self::Command => 0,
            Self::Urgent => 1,
            Self::Worker => 2,
            Self::Retirement => 3,
            Self::Native => 4,
        }
    }
    fn limit(self, limits: SupervisorLimits) -> usize {
        match self {
            Self::Command => limits.commands(),
            Self::Urgent => limits.urgent(),
            Self::Worker => limits.workers(),
            Self::Retirement => limits.retirements(),
            Self::Native => limits.native(),
        }
    }
}
struct Registry {
    phase: SupervisorPhase,
    next: u64,
    tasks: BTreeMap<u64, TaskClass>,
    counts: [usize; 5],
    failures: usize,
}
struct State {
    limits: SupervisorLimits,
    registry: Mutex<Registry>,
    changed: Notify,
}
#[derive(Clone)]
pub(super) struct TaskSupervisor {
    executor: Handle,
    state: Arc<State>,
}
impl TaskSupervisor {
    pub(super) fn new(executor: Handle, limits: SupervisorLimits) -> Self {
        Self {
            executor,
            state: Arc::new(State {
                limits,
                registry: Mutex::new(Registry {
                    phase: SupervisorPhase::Running,
                    next: 0,
                    tasks: BTreeMap::new(),
                    counts: [0; 5],
                    failures: 0,
                }),
                changed: Notify::new(),
            }),
        }
    }
    pub(super) fn workers(&self) -> usize {
        self.state.limits.workers()
    }
    pub(super) fn snapshot(&self) -> TaskSnapshot {
        let state = self.state.registry.lock();
        TaskSnapshot {
            phase: state.phase,
            outstanding: state.tasks.len(),
            failed_native_tasks: state.failures,
        }
    }
    pub(super) fn check_healthy(&self) -> Result<(), CoordinatorError> {
        let state = self.state.registry.lock();
        if state.failures > 0 {
            Err(CoordinatorError::NativeTaskFailed)
        } else {
            Ok(())
        }
    }
    pub(super) fn running(&self) -> bool {
        self.state.registry.lock().phase == SupervisorPhase::Running
    }
    fn register(&self, class: TaskClass) -> Result<TaskToken, CoordinatorError> {
        let mut state = self.state.registry.lock();
        if state.phase == SupervisorPhase::Closed
            || (state.phase == SupervisorPhase::Draining
                && !matches!(class, TaskClass::Retirement | TaskClass::Urgent))
        {
            return Err(CoordinatorError::Stopped);
        }
        if state.counts[class.index()] >= class.limit(self.state.limits) {
            return Err(CoordinatorError::Overloaded);
        }
        let id = state
            .next
            .checked_add(1)
            .ok_or(CoordinatorError::Overloaded)?;
        state.next = id;
        state.counts[class.index()] += 1;
        state.tasks.insert(id, class);
        Ok(TaskToken {
            state: self.state.clone(),
            id,
            completed: false,
        })
    }
    pub(super) fn submit<T: Send + 'static>(
        &self,
        class: TaskClass,
        work: BoxFuture<'static, Result<T, CoordinatorError>>,
    ) -> Result<BoxFuture<'static, Result<T, CoordinatorError>>, CoordinatorError> {
        let registered = self.track(class, work)?;
        let (send, receive) = oneshot::channel();
        self.executor.spawn(async move {
            let result = registered.await.into_result();
            let _ = send.send(result);
        });
        Ok(async move { receive.await.map_err(|_| CoordinatorError::Stopped)? }.boxed())
    }
    pub(super) fn track<T: Send + 'static, E: Send + 'static>(
        &self,
        class: TaskClass,
        work: BoxFuture<'static, Result<T, E>>,
    ) -> Result<RegisteredWork<T, E>, CoordinatorError> {
        Ok(RegisteredWork {
            work,
            token: Some(self.register(class)?),
        })
    }
    pub(super) fn note_failure(&self) {
        let mut state = self.state.registry.lock();
        state.failures = state.failures.saturating_add(1);
    }
    pub(super) fn retire(
        &self,
        work: BoxFuture<'static, Result<(), CoordinatorError>>,
    ) -> Result<(), CoordinatorError> {
        let work = self.track(TaskClass::Retirement, work)?;
        let state = self.state.clone();
        self.executor.spawn(async move {
            let completion = work.await;
            if completion.result().is_err() {
                let mut state = state.registry.lock();
                state.failures = state.failures.saturating_add(1);
            }
            drop(completion);
        });
        Ok(())
    }
    pub(super) fn begin_drain(&self) {
        let mut state = self.state.registry.lock();
        if state.phase == SupervisorPhase::Running {
            state.phase = SupervisorPhase::Draining;
        }
        drop(state);
        self.state.changed.notify_waiters();
    }
    /// Waits on actual task-owned tokens. Empty-to-Closed is one transition so
    /// a late cleanup cannot reopen an already reported completed drain.
    pub(super) async fn drain(&self, deadline: ShutdownDeadline) -> TaskSnapshot {
        self.begin_drain();
        loop {
            let changed = self.state.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let mut state = self.state.registry.lock();
                if state.tasks.is_empty() {
                    state.phase = SupervisorPhase::Closed;
                    return TaskSnapshot {
                        phase: state.phase,
                        outstanding: 0,
                        failed_native_tasks: state.failures,
                    };
                }
            }
            if tokio::time::timeout(deadline.remaining(), changed)
                .await
                .is_err()
            {
                return self.snapshot();
            }
        }
    }
}
impl ToolTaskSpawner for TaskSupervisor {
    fn spawn(
        &self,
        task: BoxFuture<'static, Result<(), ToolExecutionError>>,
    ) -> Result<(), ToolExecutionError> {
        let work = self
            .track(TaskClass::Native, task)
            .map_err(|error| match error {
                CoordinatorError::Overloaded => ToolExecutionError::Capacity,
                _ => ToolExecutionError::Revoked,
            })?;
        let state = self.state.clone();
        self.executor.spawn(async move {
            let completion = work.await;
            if let Err(error) = completion.result()
                && !expected_retirement(error)
            {
                let mut state = state.registry.lock();
                state.failures = state.failures.saturating_add(1);
            }
            drop(completion);
        });
        Ok(())
    }
}
/// Field drop order is intentional: a cancelled future retires its own native
/// resources while its task token is still registered, even before first poll.
pub(super) struct RegisteredWork<T, E> {
    work: BoxFuture<'static, Result<T, E>>,
    token: Option<TaskToken>,
}
impl<T, E> Future for RegisteredWork<T, E> {
    type Output = TaskCompletion<T, E>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.token.is_none() {
            return Poll::Pending;
        }
        match self.work.as_mut().poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => {
                let Some(mut token) = self.token.take() else {
                    return Poll::Pending;
                };
                token.completed = true;
                Poll::Ready(TaskCompletion {
                    result,
                    _token: token,
                })
            }
        }
    }
}
pub(super) struct TaskCompletion<T, E> {
    result: Result<T, E>,
    _token: TaskToken,
}
impl<T, E> TaskCompletion<T, E> {
    fn result(&self) -> &Result<T, E> {
        &self.result
    }
    pub(super) fn into_result(self) -> Result<T, E> {
        self.result
    }
}
struct TaskToken {
    state: Arc<State>,
    id: u64,
    completed: bool,
}
impl Drop for TaskToken {
    fn drop(&mut self) {
        let mut state = self.state.registry.lock();
        if let Some(class) = state.tasks.remove(&self.id) {
            state.counts[class.index()] -= 1;
            if !self.completed && matches!(class, TaskClass::Retirement | TaskClass::Native) {
                state.failures = state.failures.saturating_add(1);
            }
        }
        drop(state);
        self.state.changed.notify_waiters();
    }
}
fn expected_retirement(error: &ToolExecutionError) -> bool {
    if error.is_needs_unlock() {
        return true;
    }
    match error {
        ToolExecutionError::Revoked
        | ToolExecutionError::Expired
        | ToolExecutionError::StaleSession
        | ToolExecutionError::OutcomeUnknown => true,
        ToolExecutionError::Execution(cause) => matches!(
            cause.downcast_ref::<CoordinatorError>(),
            Some(
                CoordinatorError::Stopped
                    | CoordinatorError::StaleSource
                    | CoordinatorError::Expired
                    | CoordinatorError::NeedsGrant
                    | CoordinatorError::Journal(nocterm_cloud_model::journal::JournalError::Fenced)
            )
        ),
        _ => false,
    }
}
