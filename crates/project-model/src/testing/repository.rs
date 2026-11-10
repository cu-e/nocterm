use std::{
    collections::VecDeque,
    sync::{Mutex, MutexGuard},
};

use futures::channel::oneshot;

use crate::{Catalog, ProjectRepository, ProjectRepositoryOperation, ProjectStorageError};

/// One paused blocking operation, using the shared writer fixture's oneshot pattern.
/// Dropping the control handle releases the operation, including on a failed test.
pub struct RepositoryPause {
    entered: oneshot::Receiver<()>,
    release: Option<oneshot::Sender<()>>,
}

impl RepositoryPause {
    fn pending() -> (Self, PendingPause) {
        let (entered, entry) = oneshot::channel();
        let (release, resume) = oneshot::channel();
        (
            Self {
                entered: entry,
                release: Some(release),
            },
            PendingPause {
                entered,
                release: resume,
            },
        )
    }

    /// True only if the queued repository call reached its suspension point.
    pub async fn entered(&mut self) -> bool {
        (&mut self.entered).await.is_ok()
    }

    pub fn release(mut self) {
        self.resume();
    }

    fn resume(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}

impl Drop for RepositoryPause {
    fn drop(&mut self) {
        self.resume();
    }
}

struct PendingPause {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

impl PendingPause {
    fn wait(self) {
        let _ = self.entered.send(());
        let _ = futures::executor::block_on(self.release);
    }
}

struct RepositoryState {
    catalog: Option<Catalog>,
    failures: VecDeque<(ProjectRepositoryOperation, ProjectStorageError)>,
    pauses: VecDeque<(ProjectRepositoryOperation, PendingPause)>,
}

/// No workers or native I/O. The caller owns scheduling and explicit releases.
///
/// Paused calls block and must run on a real test worker, so a single-threaded
/// GPUI test executor can still drive release. Service queue tests should prefer
/// the existing asynchronous store writer fixture at that boundary.
pub struct InMemoryProjectRepository {
    state: Mutex<RepositoryState>,
}

impl InMemoryProjectRepository {
    pub fn new(catalog: Option<Catalog>) -> Self {
        Self {
            state: Mutex::new(RepositoryState {
                catalog,
                failures: VecDeque::new(),
                pauses: VecDeque::new(),
            }),
        }
    }

    pub fn snapshot(&self) -> Option<Catalog> {
        self.state().catalog.clone()
    }

    pub fn fail_next(&self, operation: ProjectRepositoryOperation, error: ProjectStorageError) {
        self.state().failures.push_back((operation, error));
    }

    pub fn pause_next(&self, operation: ProjectRepositoryOperation) -> RepositoryPause {
        let (pause, pending) = RepositoryPause::pending();
        self.state().pauses.push_back((operation, pending));
        pause
    }

    fn begin(&self, operation: ProjectRepositoryOperation) -> Result<(), ProjectStorageError> {
        let pause = {
            let mut state = self.state();
            state
                .pauses
                .iter()
                .position(|(target, _)| *target == operation)
                .and_then(|position| state.pauses.remove(position).map(|(_, pause)| pause))
        };
        if let Some(pause) = pause {
            pause.wait();
        }
        let mut state = self.state();
        if let Some(position) = state
            .failures
            .iter()
            .position(|(target, _)| *target == operation)
            && let Some((_, error)) = state.failures.remove(position)
        {
            return Err(error);
        }
        Ok(())
    }

    fn state(&self) -> MutexGuard<'_, RepositoryState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl ProjectRepository for InMemoryProjectRepository {
    fn load(&self) -> Result<Option<Catalog>, ProjectStorageError> {
        self.begin(ProjectRepositoryOperation::Load)?;
        Ok(self.snapshot())
    }

    fn save(&self, catalog: &Catalog) -> Result<(), ProjectStorageError> {
        self.begin(ProjectRepositoryOperation::Save)?;
        self.state().catalog = Some(catalog.clone());
        Ok(())
    }
}
