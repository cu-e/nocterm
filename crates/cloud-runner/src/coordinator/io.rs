//! One bounded off-thread native journal owner shared by all coordinator paths.
use futures::{FutureExt, future::BoxFuture};
use nocterm_cloud_model::{authority::StreamBinding, journal::WriterLease};
use nocterm_cloud_model::{
    coordinator::{CoordinatorError, CoordinatorJournal},
    journal::{JournalError, RunJournalCleanup},
};
use nocterm_core::{
    allocation::AllocationLease,
    persist::{InFlight, ShutdownDeadline, WriteGate},
};
use parking_lot::Mutex;
use std::{collections::BTreeMap, sync::Arc};
use tokio::{runtime::Handle, sync::Semaphore};

#[derive(Clone)]
pub(super) struct JournalIo {
    journal: Arc<dyn CoordinatorJournal>,
    cleanup: Arc<dyn RunJournalCleanup>,
    executor: Handle,
    gate: WriteGate,
    capacity: Arc<Semaphore>,
    owner: Arc<()>,
    writers: Arc<Mutex<BTreeMap<StreamBinding, WriterLease>>>,
    in_flight: InFlight,
}
pub(super) struct JournalGuard {
    publication: Arc<Publication>,
}
struct Publication {
    owner: Arc<()>,
    _guard: futures::lock::OwnedMutexGuard<()>,
    _capacity: tokio::sync::OwnedSemaphorePermit,
    _allocation: Option<AllocationLease>,
}
impl JournalIo {
    pub(super) fn new(
        journal: Arc<dyn CoordinatorJournal>,
        cleanup: Arc<dyn RunJournalCleanup>,
        executor: Handle,
    ) -> Self {
        Self {
            journal,
            cleanup,
            executor,
            gate: WriteGate::default(),
            capacity: Arc::new(Semaphore::new(256)),
            owner: Arc::new(()),
            writers: Arc::new(Mutex::new(BTreeMap::new())),
            in_flight: InFlight::default(),
        }
    }
    pub(super) fn remember_writer(&self, lease: WriterLease) -> Result<(), CoordinatorError> {
        let mut writers = self.writers.lock();
        if !writers.contains_key(lease.stream()) && writers.len() >= 4096 {
            return Err(CoordinatorError::Overloaded);
        }
        if writers
            .get(lease.stream())
            .is_some_and(|old| old.epoch() > lease.epoch())
        {
            return Err(JournalError::Fenced.into());
        }
        writers.insert(lease.stream().clone(), lease);
        Ok(())
    }
    pub(super) fn owned_writer(&self, stream: &StreamBinding) -> Option<WriterLease> {
        self.writers.lock().get(stream).cloned()
    }
    pub(super) fn journal(&self) -> &Arc<dyn CoordinatorJournal> {
        &self.journal
    }
    pub(super) async fn freeze_and_drain(&self, deadline: ShutdownDeadline) -> bool {
        self.gate.freeze();
        match tokio::time::timeout(deadline.remaining(), self.gate.owned_final_write()).await {
            Ok(_guard) => self.in_flight.count() == 0,
            Err(_) => false,
        }
    }
    pub(super) fn in_flight_writes(&self) -> usize {
        self.in_flight.count()
    }
    pub(super) async fn acquire(&self) -> Result<JournalGuard, CoordinatorError> {
        self.acquire_with(None).await
    }
    pub(super) async fn acquire_resident(
        &self,
        allocation: AllocationLease,
    ) -> Result<JournalGuard, CoordinatorError> {
        self.acquire_with(Some(allocation)).await
    }
    async fn acquire_with(
        &self,
        allocation: Option<AllocationLease>,
    ) -> Result<JournalGuard, CoordinatorError> {
        // Bound parked publication groups as well as executing native jobs.
        let capacity = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| CoordinatorError::Overloaded)?;
        let guard = self
            .gate
            .owned_normal()
            .await
            .ok_or(CoordinatorError::Stopped)?;
        Ok(JournalGuard {
            publication: Arc::new(Publication {
                owner: self.owner.clone(),
                _guard: guard,
                _capacity: capacity,
                _allocation: allocation,
            }),
        })
    }
    async fn execute<T: Send + 'static>(
        &self,
        publication: Arc<Publication>,
        operation: impl FnOnce() -> Result<T, CoordinatorError> + Send + 'static,
    ) -> Result<T, CoordinatorError> {
        let writing = self.in_flight.begin_write();
        self.executor
            .spawn_blocking(move || {
                // Both guards belong to actual native work. A cancelled receiver
                // cannot release capacity, the shared gate or in-flight accounting.
                let _publication = publication;
                let _writing = writing;
                operation()
            })
            .await
            .map_err(|cause| CoordinatorError::Backend(nocterm_core::SharedCause::new(cause)))?
    }
    pub(super) fn run<T: Send + 'static>(
        &self,
        make: impl FnOnce(Arc<dyn CoordinatorJournal>) -> BoxFuture<'static, Result<T, JournalError>>
        + Send
        + 'static,
    ) -> BoxFuture<'static, Result<T, CoordinatorError>> {
        let this = self.clone();
        async move {
            let guard = this.acquire().await?;
            this.locked(&guard, make).await
        }
        .boxed()
    }
    pub(super) async fn locked<T: Send + 'static>(
        &self,
        guard: &JournalGuard,
        make: impl FnOnce(Arc<dyn CoordinatorJournal>) -> BoxFuture<'static, Result<T, JournalError>>
        + Send
        + 'static,
    ) -> Result<T, CoordinatorError> {
        if !Arc::ptr_eq(&self.owner, &guard.publication.owner) {
            return Err(CoordinatorError::Conflict);
        }
        let journal = self.journal.clone();
        self.execute(guard.publication.clone(), move || {
            futures::executor::block_on(make(journal)).map_err(CoordinatorError::from)
        })
        .await
    }
    pub(super) async fn cleanup_locked<T: Send + 'static>(
        &self,
        guard: &JournalGuard,
        make: impl FnOnce(Arc<dyn RunJournalCleanup>) -> BoxFuture<'static, Result<T, JournalError>>
        + Send
        + 'static,
    ) -> Result<T, CoordinatorError> {
        if !Arc::ptr_eq(&self.owner, &guard.publication.owner) {
            return Err(CoordinatorError::Conflict);
        }
        let cleanup = self.cleanup.clone();
        self.execute(guard.publication.clone(), move || {
            futures::executor::block_on(make(cleanup)).map_err(CoordinatorError::from)
        })
        .await
    }
    pub(super) async fn session_loss_locked(
        &self,
        guard: &JournalGuard,
        cleanup: Arc<dyn nocterm_cloud_model::journal::RunSessionFactCleanup>,
        batch: nocterm_cloud_model::journal::SessionLossCleanup,
        allocation: AllocationLease,
    ) -> Result<(), CoordinatorError> {
        if !Arc::ptr_eq(&self.owner, &guard.publication.owner) {
            return Err(CoordinatorError::Conflict);
        }
        self.execute(guard.publication.clone(), move || {
            futures::executor::block_on(cleanup.finish_session_loss(batch, allocation))
                .map_err(CoordinatorError::from)
        })
        .await
    }
    pub(super) fn tool<T: Send + 'static>(
        &self,
        make: impl FnOnce(
            Arc<dyn CoordinatorJournal>,
        ) -> BoxFuture<
            'static,
            Result<T, nocterm_cloud_model::tool::ToolExecutionError>,
        > + Send
        + 'static,
    ) -> BoxFuture<'static, Result<T, CoordinatorError>> {
        let this = self.clone();
        async move {
            let guard = this.acquire().await?;
            let journal = this.journal.clone();
            this.execute(guard.publication.clone(), move || {
                futures::executor::block_on(make(journal)).map_err(CoordinatorError::from)
            })
            .await
        }
        .boxed()
    }
}
