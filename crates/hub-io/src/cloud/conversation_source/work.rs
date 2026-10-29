//! Shared finite blocking admission; cancellation never frees a physical read early.
use nocterm_cloud_model::projection::ProjectionError;
use nocterm_core::allocation::{Allocated, AllocationLease};
use std::sync::Arc;
use std::{future::Future, pin::Pin};
use tokio::{
    runtime::Handle,
    sync::{OwnedSemaphorePermit, Semaphore},
    task::JoinHandle,
};

pub struct ConversationWorkers {
    runtime: Handle,
    normal: Arc<Semaphore>,
    urgent: Arc<Semaphore>,
}
impl ConversationWorkers {
    /// Inject one shared owner across sources. Idle chats own no worker slot.
    pub fn new(runtime: Handle, maximum: usize) -> Result<Arc<Self>, ProjectionError> {
        if !(2..=32).contains(&maximum) {
            return Err(ProjectionError::Capacity);
        }
        Ok(Arc::new(Self {
            runtime,
            normal: Arc::new(Semaphore::new(maximum - 1)),
            urgent: Arc::new(Semaphore::new(1)),
        }))
    }
    pub(super) fn task<
        T: Send + 'static,
        F: Future<Output = Result<T, ProjectionError>> + Send + 'static,
    >(
        &self,
        allocation: AllocationLease,
        future: F,
    ) -> crate::task::Spawned<NativeResult<T>> {
        let mut operation = TaskOperation {
            future: Box::pin(future),
            allocation,
        };
        crate::task::spawn(&self.runtime, async move { operation.run().await })
    }
    pub(super) fn blocking<T: Send + 'static>(
        &self,
        allocation: AllocationLease,
        urgent: bool,
        task: impl FnOnce() -> Result<T, ProjectionError> + Send + 'static,
    ) -> Result<JoinHandle<BlockingResult<T>>, ProjectionError> {
        let slot = (if urgent { &self.urgent } else { &self.normal })
            .clone()
            .try_acquire_owned()
            .map_err(|_| ProjectionError::Capacity)?;
        Ok(self.runtime.spawn_blocking(move || BlockingResult {
            value: NativeResult {
                value: task(),
                allocation,
            },
            _slot: slot,
        }))
    }
}
pub(super) struct BlockingResult<T> {
    value: NativeResult<T>,
    _slot: OwnedSemaphorePermit,
}
impl<T> BlockingResult<T> {
    pub(super) fn into_allocated(self) -> Result<Allocated<T>, ProjectionError> {
        self.value.into_allocated()
    }
    pub(super) fn result(&self) -> Result<&T, ProjectionError> {
        self.value.value.as_ref().map_err(Clone::clone)
    }
}

/// Data is destroyed before its real original allowance, including an unobserved
/// completion and cancellation. Transfer always returns Allocated<T>, never T.
pub(super) struct NativeResult<T> {
    value: Result<T, ProjectionError>,
    allocation: AllocationLease,
}
impl<T> NativeResult<T> {
    pub(super) fn into_allocated(self) -> Result<Allocated<T>, ProjectionError> {
        let Self { value, allocation } = self;
        value.map(|value| Allocated::new(value, allocation))
    }
}
struct TaskOperation<F> {
    future: Pin<Box<F>>,
    allocation: AllocationLease,
}
impl<T, F: Future<Output = Result<T, ProjectionError>>> TaskOperation<F> {
    async fn run(&mut self) -> NativeResult<T> {
        let value = self.future.as_mut().await;
        NativeResult {
            value,
            allocation: self.allocation.clone(),
        }
    }
}

/// Real sub-admission inside a single already reserved native allowance. SDK
/// body/decoded owners retain each slice; a later operation cannot reuse it
/// merely because the caller future completed. No new global quota is created.
pub(super) struct WorkSlices {
    limit: usize,
    used: std::sync::Mutex<usize>,
    _parent: AllocationLease,
}
impl WorkSlices {
    pub(super) fn new(parent: AllocationLease, limit: usize) -> Result<Arc<Self>, ProjectionError> {
        if limit == 0 || limit > parent.bytes() || parent.payloads() == 0 {
            return Err(ProjectionError::Capacity);
        }
        Ok(Arc::new(Self {
            limit,
            used: std::sync::Mutex::new(0),
            _parent: parent,
        }))
    }
    pub(super) fn reserve(
        self: &Arc<Self>,
        bytes: usize,
    ) -> Result<AllocationLease, ProjectionError> {
        let mut used = self.used.lock().map_err(|_| ProjectionError::Unavailable)?;
        let next = used
            .checked_add(bytes)
            .filter(|next| *next <= self.limit)
            .ok_or(ProjectionError::Capacity)?;
        *used = next;
        Ok(AllocationLease::new(
            bytes,
            1,
            WorkSlice {
                owner: self.clone(),
                bytes,
            },
        ))
    }
}
struct WorkSlice {
    owner: Arc<WorkSlices>,
    bytes: usize,
}
impl Drop for WorkSlice {
    fn drop(&mut self) {
        let mut used = self
            .owner
            .used
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *used = used.saturating_sub(self.bytes);
    }
}

impl NativeResult<super::replay::NativePoll> {
    pub(super) fn into_replay(
        self,
    ) -> Result<nocterm_cloud_model::projection::ports::ConversationReplay, ProjectionError> {
        let Self { value, allocation } = self;
        Ok(match value? {
            super::replay::NativePoll::Page(batch) => {
                nocterm_cloud_model::projection::ports::ConversationReplay::Page(Box::new(
                    Allocated::new(*batch, allocation),
                ))
            }
            super::replay::NativePoll::Readmission => {
                nocterm_cloud_model::projection::ports::ConversationReplay::Readmission
            }
        })
    }
}
