//! Shared captured-source environment scripts; suspended replies preserve cancellation tests.
use crate::{
    EnvironmentError, EnvironmentFuture, EnvironmentSnapshot, EnvironmentSource, LanguageSelection,
    SetupProgress, SetupUpdate, WorkspaceEnvironment,
};
use futures::{FutureExt as _, future::BoxFuture};
use nocterm_session::local_fs::CancelFlag;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
type Step = (
    Option<BoxFuture<'static, ()>>,
    Result<EnvironmentSnapshot, EnvironmentError>,
);
#[derive(Clone)]
pub struct EnvironmentCall {
    pub source: EnvironmentSource,
    pub selections: Option<Vec<LanguageSelection>>,
    pub cancel: CancelFlag,
}
#[derive(Default)]
struct Script {
    inspections: VecDeque<Step>,
    installations: VecDeque<Step>,
    calls: Vec<EnvironmentCall>,
    updates: VecDeque<Vec<SetupUpdate>>,
}
#[derive(Clone, Default)]
pub struct FakeWorkspaceEnvironment {
    state: Arc<Mutex<Script>>,
}
impl FakeWorkspaceEnvironment {
    pub fn progress(&self, updates: Vec<SetupUpdate>) -> Result<(), EnvironmentError> {
        self.state
            .lock()
            .map_err(|_| EnvironmentError::Canceled)?
            .updates
            .push_back(updates);
        Ok(())
    }

    pub fn respond_inspect(
        &self,
        result: Result<EnvironmentSnapshot, EnvironmentError>,
    ) -> Result<(), EnvironmentError> {
        self.enqueue(false, None, result)
    }
    pub fn respond_install(
        &self,
        result: Result<EnvironmentSnapshot, EnvironmentError>,
    ) -> Result<(), EnvironmentError> {
        self.enqueue(true, None, result)
    }
    pub fn inspect_after(
        &self,
        gate: BoxFuture<'static, ()>,
        result: Result<EnvironmentSnapshot, EnvironmentError>,
    ) -> Result<(), EnvironmentError> {
        self.enqueue(false, Some(gate), result)
    }
    pub fn install_after(
        &self,
        gate: BoxFuture<'static, ()>,
        result: Result<EnvironmentSnapshot, EnvironmentError>,
    ) -> Result<(), EnvironmentError> {
        self.enqueue(true, Some(gate), result)
    }
    pub fn calls(&self) -> Result<Vec<EnvironmentCall>, EnvironmentError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| EnvironmentError::Canceled)?
            .calls
            .clone())
    }
    fn enqueue(
        &self,
        install: bool,
        gate: Option<BoxFuture<'static, ()>>,
        result: Result<EnvironmentSnapshot, EnvironmentError>,
    ) -> Result<(), EnvironmentError> {
        let mut state = self.state.lock().map_err(|_| EnvironmentError::Canceled)?;
        if install {
            state.installations.push_back((gate, result));
        } else {
            state.inspections.push_back((gate, result));
        }
        Ok(())
    }
    fn run(
        &self,
        source: EnvironmentSource,
        selections: Option<Vec<LanguageSelection>>,
        cancel: CancelFlag,
        progress: Option<SetupProgress>,
    ) -> EnvironmentFuture<EnvironmentSnapshot> {
        let state = self.state.clone();
        async move {
            let (step, updates) = {
                let mut state = state.lock().map_err(|_| EnvironmentError::Canceled)?;
                let install = selections.is_some();
                state.calls.push(EnvironmentCall {
                    source,
                    selections,
                    cancel,
                });
                let step = (if install {
                    &mut state.installations
                } else {
                    &mut state.inspections
                })
                .pop_front()
                .unwrap_or((None, Err(EnvironmentError::Unavailable)));
                let updates = if install {
                    state.updates.pop_front().unwrap_or_default()
                } else {
                    vec![]
                };
                (step, updates)
            };
            if let Some(progress) = progress {
                for update in updates {
                    progress(update);
                }
            }
            let (gate, result) = step;
            if let Some(gate) = gate {
                gate.await;
            }
            result
        }
        .boxed()
    }
}
impl WorkspaceEnvironment for FakeWorkspaceEnvironment {
    fn inspect(
        &self,
        source: EnvironmentSource,
        cancel: CancelFlag,
    ) -> EnvironmentFuture<EnvironmentSnapshot> {
        self.run(source, None, cancel, None)
    }
    fn install(
        &self,
        source: EnvironmentSource,
        selections: Vec<LanguageSelection>,
        cancel: CancelFlag,
        progress: SetupProgress,
    ) -> EnvironmentFuture<EnvironmentSnapshot> {
        self.run(source, Some(selections), cancel, Some(progress))
    }
}
