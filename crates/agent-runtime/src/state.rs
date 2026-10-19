//! Optimistic preferences use Store's shared snapshot driver.
use super::{AgentPersistenceError, AgentStorage, Runtime};
use futures::future::BoxFuture;
use gpui::{App, AppContext as _, Context, Entity, Subscription};
use nocterm_ai::favorites::AgentStateFile;
use nocterm_store::{Persisted, Publication, StoreError};

pub(super) struct AgentStateStore {
    worker: Entity<Persisted<AgentStateFile>>,
    initial_error: Option<AgentPersistenceError>,
    _observer: Subscription,
}
impl AgentStateStore {
    pub(super) fn new(storage: &AgentStorage, cx: &mut Context<Runtime>) -> Self {
        let repository = storage.state();
        let initial = storage.initial();
        let worker = cx.new(|_| {
            Persisted::new(
                initial,
                std::sync::Arc::new(move |state| {
                    let repository = repository.clone();
                    Box::pin(
                        async move { repository.save(state).await.map_err(StoreError::backend) },
                    )
                }),
                Publication::Optimistic,
                32,
            )
        });
        let observer = cx.observe(&worker, |_, _, cx| cx.notify());
        Self {
            worker,
            initial_error: storage.initial_error().map(Into::into),
            _observer: observer,
        }
    }
    pub(super) fn visible<'a>(&'a self, cx: &'a App) -> &'a AgentStateFile {
        self.worker.read(cx).visible()
    }
    pub(super) fn error(&self, cx: &App) -> Option<AgentPersistenceError> {
        self.worker
            .read(cx)
            .error()
            .cloned()
            .map(Into::into)
            .or_else(|| self.initial_error.clone())
    }
    pub(super) fn edit(&mut self, edit: impl FnOnce(&mut AgentStateFile), cx: &mut App) {
        let mut desired = self.visible(cx).clone();
        edit(&mut desired);
        if desired == *self.visible(cx) {
            return;
        }
        self.initial_error = None;
        if let Err(error) = self.worker.update(cx, |worker, cx| {
            worker.update_latest(move |state| *state = desired, cx)
        }) {
            tracing::warn!(%error,"agent preferences admission failed")
        }
    }
    pub(super) fn capture(&mut self, cx: &mut App) -> BoxFuture<'static, Result<(), StoreError>> {
        self.worker
            .update(cx, |worker, _| worker.capture_shutdown())
    }
}
