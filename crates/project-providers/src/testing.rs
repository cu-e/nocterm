//! Deterministic shared preparation port fake, enabled only by dev-dependencies.
use crate::{
    EngineInventory, OwnedContainer, PreparedProject, ProjectPreparation, ProjectSetupError,
    ProjectSetupRequest, SetupFuture, SetupProgress, SetupUpdate,
};
use futures::{FutureExt as _, future::BoxFuture};
use nocterm_session::{HostExec, local_fs::CancelFlag};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct PreparationScript {
    results: VecDeque<Result<PreparedProject, ProjectSetupError>>,
    discard_results: VecDeque<Result<(), ProjectSetupError>>,
    requests: Vec<ProjectSetupRequest>,
    discarded: Vec<OwnedContainer>,
    inventories: VecDeque<Vec<EngineInventory>>,
    discovered: Vec<Arc<dyn HostExec>>,
    prepared: Vec<Arc<dyn HostExec>>,
    updates: VecDeque<Vec<SetupUpdate>>,
    gate: Option<BoxFuture<'static, ()>>,
}
#[derive(Clone, Default)]
pub struct FakeProjectPreparation {
    state: Arc<Mutex<PreparationScript>>,
}
impl FakeProjectPreparation {
    pub fn progress(&self, updates: Vec<SetupUpdate>) -> Result<(), ProjectSetupError> {
        self.state
            .lock()
            .map_err(|_| ProjectSetupError::Canceled)?
            .updates
            .push_back(updates);
        Ok(())
    }
    pub fn inventory(&self, inventory: Vec<EngineInventory>) -> Result<(), ProjectSetupError> {
        self.state
            .lock()
            .map_err(|_| ProjectSetupError::Canceled)?
            .inventories
            .push_back(inventory);
        Ok(())
    }
    pub fn discovered(&self) -> Result<Vec<Arc<dyn HostExec>>, ProjectSetupError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| ProjectSetupError::Canceled)?
            .discovered
            .clone())
    }
    pub fn prepared(&self) -> Result<Vec<Arc<dyn HostExec>>, ProjectSetupError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| ProjectSetupError::Canceled)?
            .prepared
            .clone())
    }
    pub fn respond(
        &self,
        result: Result<PreparedProject, ProjectSetupError>,
    ) -> Result<(), ProjectSetupError> {
        self.state
            .lock()
            .map_err(|_| ProjectSetupError::Canceled)?
            .results
            .push_back(result);
        Ok(())
    }
    pub fn discard_result(
        &self,
        result: Result<(), ProjectSetupError>,
    ) -> Result<(), ProjectSetupError> {
        self.state
            .lock()
            .map_err(|_| ProjectSetupError::Canceled)?
            .discard_results
            .push_back(result);
        Ok(())
    }
    pub fn suspend(&self, gate: BoxFuture<'static, ()>) -> Result<(), ProjectSetupError> {
        self.state
            .lock()
            .map_err(|_| ProjectSetupError::Canceled)?
            .gate = Some(gate);
        Ok(())
    }
    pub fn requests(&self) -> Result<Vec<ProjectSetupRequest>, ProjectSetupError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| ProjectSetupError::Canceled)?
            .requests
            .clone())
    }
    pub fn discarded(&self) -> Result<Vec<OwnedContainer>, ProjectSetupError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| ProjectSetupError::Canceled)?
            .discarded
            .clone())
    }
}
impl ProjectPreparation for FakeProjectPreparation {
    fn discover(&self, exec: Arc<dyn HostExec>) -> SetupFuture<Vec<EngineInventory>> {
        let state = self.state.clone();
        async move {
            let mut state = state.lock().map_err(|_| ProjectSetupError::Canceled)?;
            state.discovered.push(exec);
            Ok(state.inventories.pop_front().unwrap_or_default())
        }
        .boxed()
    }
    fn prepare(
        &self,
        exec: Arc<dyn HostExec>,
        request: ProjectSetupRequest,
        _: CancelFlag,
        progress: SetupProgress,
    ) -> SetupFuture<PreparedProject> {
        let state = self.state.clone();
        async move {
            let (gate, result, updates) = {
                let mut state = state.lock().map_err(|_| ProjectSetupError::Canceled)?;
                state.requests.push(request);
                state.prepared.push(exec);
                (
                    state.gate.take(),
                    state
                        .results
                        .pop_front()
                        .unwrap_or(Err(ProjectSetupError::InvalidInput(
                            "no scripted preparation result",
                        ))),
                    state.updates.pop_front().unwrap_or_default(),
                )
            };
            for update in updates {
                progress(update);
            }
            if let Some(gate) = gate {
                gate.await;
            }
            result
        }
        .boxed()
    }
    fn discard_owned(&self, _: Arc<dyn HostExec>, receipt: OwnedContainer) -> SetupFuture<()> {
        let state = self.state.clone();
        async move {
            let mut state = state.lock().map_err(|_| ProjectSetupError::Canceled)?;
            state.discarded.push(receipt);
            state.discard_results.pop_front().unwrap_or(Ok(()))
        }
        .boxed()
    }
}

mod image_search;
pub use image_search::{FakeImageSearch, ImageSearchCall};
mod environment;
pub use environment::{CreatedEnvironmentCall, EnvironmentCall, FakeWorkspaceEnvironment};

mod removal;
pub use removal::{ContainerActionCall, FakeWorkspaceContainerActions};
