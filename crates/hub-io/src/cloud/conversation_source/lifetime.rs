//! A logical head refusal wakes observers and fences later bounded publication.
use nocterm_cloud_model::projection::ProjectionError;
use std::sync::Mutex;
use tokio::sync::watch;
pub(super) struct SourceLifetime {
    gate: Mutex<()>,
    notice: watch::Sender<Option<ProjectionError>>,
}
impl SourceLifetime {
    pub(super) fn new() -> Self {
        Self {
            gate: Mutex::new(()),
            notice: watch::channel(None).0,
        }
    }
    pub(super) fn refuse(&self, error: &ProjectionError) {
        if !matches!(
            error,
            ProjectionError::Stale
                | ProjectionError::WrongSource
                | ProjectionError::Rollback
                | ProjectionError::Discontinuous
                | ProjectionError::InvalidRecord(_)
                | ProjectionError::Unsupported
        ) {
            return;
        }
        let _gate = self
            .gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.notice.send_if_modified(|old| {
            if old.is_some() {
                false
            } else {
                *old = Some(error.clone());
                true
            }
        });
    }
    pub(super) fn subscribe(&self) -> watch::Receiver<Option<ProjectionError>> {
        self.notice.subscribe()
    }
    fn current(&self) -> Result<(), ProjectionError> {
        self.notice.borrow().clone().map_or(Ok(()), Err)
    }
    fn observed<T>(&self, result: Result<T, ProjectionError>) -> Result<T, ProjectionError> {
        result.inspect_err(|error| self.refuse(error))
    }
    /// Normal direct checks notify permanent refusal immediately. Callers must
    /// not already hold the lifetime/native publication guard.
    pub(super) fn check<T>(
        &self,
        original: impl FnOnce() -> Result<T, ProjectionError>,
    ) -> Result<T, ProjectionError> {
        self.observed(self.current().and_then(|()| original()))
    }
    /// Raw validation under the guard cannot notify recursively. Both this guard
    /// and any original-member guard in action are gone before refusal is latched.
    pub(super) fn publish<T>(
        &self,
        original: impl FnOnce() -> Result<(), ProjectionError>,
        action: impl FnOnce() -> Result<T, ProjectionError>,
    ) -> Result<T, ProjectionError> {
        let result = (|| {
            let _guard = self.gate.lock().map_err(|_| ProjectionError::Unavailable)?;
            self.current()?;
            original()?;
            action()
        })();
        self.observed(result)
    }
    pub(super) async fn ended(
        &self,
        original: impl FnOnce() -> Result<(), ProjectionError>,
        native: impl std::future::Future<Output = ()>,
    ) {
        let mut notice = self.subscribe();
        // Subscribe before the first check, which can itself discover refusal.
        if self.check(original).is_err() {
            return;
        }
        tokio::select! { _ = native => (), _ = notice.changed() => () }
    }
}
