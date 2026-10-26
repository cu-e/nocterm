//! Retained non-destructive environment jobs; dropping a view never retargets admitted work.
mod admission;
pub(crate) use admission::EnvironmentAdmission;

use futures::{FutureExt as _, channel::oneshot};
use gpui::BackgroundExecutor;
use nocterm_project_providers::{
    EnvironmentError, EnvironmentFuture, EnvironmentSnapshot, EnvironmentSource, LanguageSelection,
    SetupProgress, SetupUpdate, WorkspaceEnvironment,
};
use nocterm_session::local_fs::CancelFlag;
use std::sync::{Arc, Mutex, Weak};
#[derive(Default)]
struct AdmissionState {
    closing: bool,
    operations: Vec<Weak<EnvironmentOwner>>,
    busy: Vec<nocterm_project_providers::EnvironmentKey>,
}

#[derive(Clone)]
pub struct EnvironmentOperation(Arc<EnvironmentOwner>);
struct EnvironmentOwner {
    port: Arc<dyn WorkspaceEnvironment>,
    source: EnvironmentSource,
    executor: BackgroundExecutor,
    cancel: CancelFlag,
    state: Mutex<OperationState>,
    admission: Arc<Mutex<AdmissionState>>,
    changed: event_listener::Event,
}
#[derive(Default)]
struct OperationState {
    running: bool,
    snapshot: Option<EnvironmentSnapshot>,
    error: Option<EnvironmentError>,
    progress: Option<SetupUpdate>,
}
impl EnvironmentOperation {
    pub fn inspect(&self) -> EnvironmentFuture<EnvironmentSnapshot> {
        self.start(None, Arc::new(|_| {}))
    }
    pub fn install(
        &self,
        selections: Vec<LanguageSelection>,
        progress: SetupProgress,
    ) -> EnvironmentFuture<EnvironmentSnapshot> {
        self.start(Some(selections), progress)
    }
    pub fn cancel(&self) {
        self.0.cancel.cancel();
    }
    pub fn is_running(&self) -> bool {
        self.0.state.lock().is_ok_and(|state| state.running)
    }
    pub fn snapshot(&self) -> Option<EnvironmentSnapshot> {
        self.0.state.lock().ok()?.snapshot.clone()
    }
    pub fn error(&self) -> Option<EnvironmentError> {
        self.0.state.lock().ok()?.error.clone()
    }
    pub fn progress(&self) -> Option<SetupUpdate> {
        self.0.state.lock().ok()?.progress.clone()
    }
    fn start(
        &self,
        selections: Option<Vec<LanguageSelection>>,
        progress: SetupProgress,
    ) -> EnvironmentFuture<EnvironmentSnapshot> {
        if let Err(error) = self.reserve(selections.as_deref()) {
            return async move { Err(error) }.boxed();
        }
        let mutating = selections.is_some();
        let (sender, receiver) = oneshot::channel();
        let operation = self.clone();
        self.0
            .executor
            .spawn(async move {
                let owner = &operation.0;
                let result = if let Some(selections) = selections {
                    let observed = operation.clone();
                    let callback = Arc::new(move |update: SetupUpdate| {
                        if let Ok(mut state) = observed.0.state.lock() {
                            state.progress = Some(update.clone());
                        }
                        progress(update);
                    });
                    owner
                        .port
                        .install(
                            owner.source.clone(),
                            selections,
                            owner.cancel.clone(),
                            callback,
                        )
                        .await
                } else {
                    owner
                        .port
                        .inspect(owner.source.clone(), owner.cancel.clone())
                        .await
                };
                let result = if owner.cancel.is_cancelled() && result.is_ok() {
                    Err(EnvironmentError::Canceled)
                } else {
                    result
                };
                if let Ok(mut state) = owner.state.lock() {
                    match &result {
                        Ok(snapshot) => {
                            state.snapshot = Some(snapshot.clone());
                            state.error = None;
                        }
                        Err(error) => state.error = Some(error.clone()),
                    }
                    state.running = false;
                    state.progress = None;
                }
                if mutating && let Ok(mut admission) = owner.admission.lock() {
                    admission.busy.retain(|key| key != &owner.source.key());
                }
                owner.changed.notify(usize::MAX);
                let _ = sender.send(result);
            })
            .detach();
        async move { receiver.await.unwrap_or(Err(EnvironmentError::Canceled)) }.boxed()
    }
    fn reserve(&self, selections: Option<&[LanguageSelection]>) -> Result<(), EnvironmentError> {
        if self.0.cancel.is_cancelled() {
            return Err(EnvironmentError::Canceled);
        }
        if let Some(selections) = selections {
            nocterm_project_providers::validate_language_selections(selections)?;
        }
        let mut admission = self
            .0
            .admission
            .lock()
            .map_err(|_| EnvironmentError::Canceled)?;
        let mut state = self
            .0
            .state
            .lock()
            .map_err(|_| EnvironmentError::Canceled)?;
        if admission.closing {
            return Err(EnvironmentError::Unavailable);
        }
        if state.running || selections.is_some() && admission.busy.contains(&self.0.source.key()) {
            return Err(EnvironmentError::Busy);
        }
        if selections.is_some() {
            admission.busy.push(self.0.source.key());
        }
        state.running = true;
        state.progress = None;
        state.error = None;
        Ok(())
    }
    async fn drain(&self) {
        loop {
            let listener = self.0.changed.listen();
            if !self.is_running() {
                return;
            }
            listener.await;
        }
    }
}
