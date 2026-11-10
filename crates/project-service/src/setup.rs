//! One admitted preparation retains its exact host and prepared resource for save retry.
mod admission;
pub(crate) use admission::SetupAdmission;
use nocterm_project_model::ProjectTarget;
use nocterm_project_providers::{
    PreparedProject, ProjectPreparation, ProjectSetupError, ProjectSetupRequest, SetupProgress,
};
use nocterm_session::{HostExec, local_fs::CancelFlag};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct WorkspaceSetup(Arc<SetupOwner>);
struct SetupOwner {
    preparation: Arc<dyn ProjectPreparation>,
    exec: Arc<dyn HostExec>,
    request: ProjectSetupRequest,
    cancel: CancelFlag,
    state: Mutex<SetupState>,
    changed: event_listener::Event,
}
#[derive(Default)]
struct SetupState {
    running: bool,
    prepared: Option<PreparedProject>,
    committed: bool,
    publishing: bool,
}
impl WorkspaceSetup {
    pub fn new(
        preparation: Arc<dyn ProjectPreparation>,
        exec: Arc<dyn HostExec>,
        request: ProjectSetupRequest,
    ) -> Self {
        Self(Arc::new(SetupOwner {
            preparation,
            exec,
            request,
            cancel: CancelFlag::default(),
            state: Mutex::new(SetupState::default()),
            changed: event_listener::Event::new(),
        }))
    }
    pub async fn prepare(
        &self,
        progress: SetupProgress,
    ) -> Result<ProjectTarget, ProjectSetupError> {
        {
            let mut state = self
                .0
                .state
                .lock()
                .map_err(|_| ProjectSetupError::Canceled)?;
            if self.0.cancel.is_cancelled() {
                return Err(ProjectSetupError::Canceled);
            }
            if let Some(prepared) = &state.prepared {
                return Ok(prepared.target.clone());
            }
            if state.running {
                return Err(ProjectSetupError::InvalidInput(
                    "workspace preparation is already running",
                ));
            }
            state.running = true;
        }
        let result = self
            .0
            .preparation
            .prepare(
                self.0.exec.clone(),
                self.0.request.clone(),
                self.0.cancel.clone(),
                progress,
            )
            .await;
        let cancelled = self.0.cancel.is_cancelled();
        let mut cleanup = None;
        let result = {
            let mut state = self
                .0
                .state
                .lock()
                .map_err(|_| ProjectSetupError::Canceled)?;
            match result {
                Ok(prepared) if cancelled => {
                    cleanup = prepared.owned;
                    Err(ProjectSetupError::Canceled)
                }
                Ok(prepared) => {
                    let target = prepared.target.clone();
                    state.prepared = Some(prepared);
                    Ok(target)
                }
                Err(error) => Err(error),
            }
        };
        let result = if let Some(owned) = cleanup {
            self.0
                .preparation
                .discard_owned(self.0.exec.clone(), owned)
                .await
                .and(result)
        } else {
            result
        };
        if let Ok(mut state) = self.0.state.lock() {
            state.running = false;
        }
        self.0.changed.notify(usize::MAX);
        result
    }
    pub fn is_prepared(&self) -> bool {
        self.0
            .state
            .lock()
            .is_ok_and(|state| state.prepared.is_some())
    }
    pub fn publishing(&self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.publishing = true;
        }
    }
    pub fn publication_failed(&self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.publishing = false;
        }
        self.0.changed.notify(usize::MAX);
    }
    /// Call only after the catalog publication succeeds.
    pub fn committed(&self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.committed = true;
        }
        self.0.changed.notify(usize::MAX);
    }
    async fn drain(&self) -> Result<(), ProjectSetupError> {
        loop {
            let changed = self.0.changed.listen();
            let finished = {
                let state = self
                    .0
                    .state
                    .lock()
                    .map_err(|_| ProjectSetupError::Canceled)?;
                !state.running && (!state.publishing || state.committed)
            };
            if finished {
                return self.discard().await;
            }
            changed.await;
        }
    }
    /// Explicit dismissal retires admission and deletes only owned, unsaved containers.
    pub async fn discard(&self) -> Result<(), ProjectSetupError> {
        self.0.cancel.cancel();
        let owned = {
            let mut state = self
                .0
                .state
                .lock()
                .map_err(|_| ProjectSetupError::Canceled)?;
            if state.committed || state.publishing {
                return Ok(());
            }
            state.prepared.take().and_then(|prepared| prepared.owned)
        };
        if let Some(owned) = owned {
            self.0
                .preparation
                .discard_owned(self.0.exec.clone(), owned)
                .await?;
        }
        Ok(())
    }
}
