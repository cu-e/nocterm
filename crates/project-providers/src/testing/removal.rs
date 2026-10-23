//! Shared scripted explicit container-action capability.
use crate::*;
use futures::{FutureExt as _, future::BoxFuture};
use nocterm_session::local_fs::CancelFlag;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct ContainerActionCall {
    pub source: ContainerActionSource,
    pub action: ContainerAction,
    pub cancel: CancelFlag,
}
#[derive(Default)]
struct Script {
    calls: Vec<ContainerActionCall>,
    results: VecDeque<Result<ContainerActionOutcome, ContainerActionError>>,
    gates: VecDeque<BoxFuture<'static, ()>>,
}
#[derive(Clone, Default)]
pub struct FakeWorkspaceContainerActions(Arc<Mutex<Script>>);
impl FakeWorkspaceContainerActions {
    pub fn respond(
        &self,
        result: Result<ContainerActionOutcome, ContainerActionError>,
    ) -> Result<(), ContainerActionError> {
        self.0
            .lock()
            .map_err(|_| ContainerActionError::Canceled)?
            .results
            .push_back(result);
        Ok(())
    }
    pub fn suspend(&self, gate: BoxFuture<'static, ()>) -> Result<(), ContainerActionError> {
        self.0
            .lock()
            .map_err(|_| ContainerActionError::Canceled)?
            .gates
            .push_back(gate);
        Ok(())
    }
    pub fn calls(&self) -> Result<Vec<ContainerActionCall>, ContainerActionError> {
        Ok(self
            .0
            .lock()
            .map_err(|_| ContainerActionError::Canceled)?
            .calls
            .clone())
    }
}
impl WorkspaceContainerActions for FakeWorkspaceContainerActions {
    fn apply(
        &self,
        source: ContainerActionSource,
        action: ContainerAction,
        cancel: CancelFlag,
    ) -> ContainerActionFuture {
        let script = self.0.clone();
        async move {
            let (gate, result) = {
                let mut script = script.lock().map_err(|_| ContainerActionError::Canceled)?;
                script.calls.push(ContainerActionCall {
                    source,
                    action,
                    cancel,
                });
                (
                    script.gates.pop_front(),
                    script
                        .results
                        .pop_front()
                        .unwrap_or(Err(ContainerActionError::Unavailable)),
                )
            };
            if let Some(gate) = gate {
                gate.await;
            }
            result
        }
        .boxed()
    }
}
