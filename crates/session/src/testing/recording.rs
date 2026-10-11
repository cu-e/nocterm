//! Explicitly driven recording ports: no files, runtime, clocks or worker threads.
use super::{Gate, lock};
use crate::recording::{
    AppendBlock, AppendOutcome, OutputLog, OutputLogFactory, PhysicalPrefix, RecordingError,
    RecordingOptions, SealOutcome,
};
use futures::{FutureExt, future::BoxFuture};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

type Creation = Result<Arc<dyn OutputLog>, RecordingError>;
type CreationStep = (Gate, Creation);
#[derive(Clone, Default)]
pub struct ScriptedOutputLogFactory {
    state: Arc<Mutex<FactoryState>>,
}
#[derive(Default)]
struct FactoryState {
    requests: Vec<RecordingOptions>,
    steps: VecDeque<CreationStep>,
}
impl ScriptedOutputLogFactory {
    pub fn enqueue(&self, gate: Gate, result: Creation) {
        lock(&self.state).steps.push_back((gate, result));
    }
    pub fn requests(&self) -> Vec<RecordingOptions> {
        lock(&self.state).requests.clone()
    }
}
impl OutputLogFactory for ScriptedOutputLogFactory {
    fn create(&self, options: RecordingOptions) -> BoxFuture<'static, Creation> {
        let state = self.state.clone();
        async move {
            let step = {
                let mut state = lock(&state);
                state.requests.push(options);
                state.steps.pop_front()
            };
            let Some((gate, result)) = step else {
                return Err(RecordingError::Unavailable);
            };
            gate.wait().await;
            result
        }
        .boxed()
    }
}
#[derive(Clone)]
pub struct ScriptedOutputLog {
    path: PathBuf,
    state: Arc<Mutex<LogState>>,
}
#[derive(Default)]
struct LogState {
    requests: Vec<AppendBlock>,
    seals: usize,
    append_steps: VecDeque<(Gate, AppendOutcome)>,
    seal_steps: VecDeque<(Gate, SealOutcome)>,
    prefix: PhysicalPrefix,
}
impl ScriptedOutputLog {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            state: Arc::default(),
        }
    }
    pub fn enqueue_append(&self, gate: Gate, outcome: AppendOutcome) {
        lock(&self.state).append_steps.push_back((gate, outcome));
    }
    pub fn enqueue_seal(&self, gate: Gate, outcome: SealOutcome) {
        lock(&self.state).seal_steps.push_back((gate, outcome));
    }
    pub fn requests(&self) -> Vec<AppendBlock> {
        lock(&self.state).requests.clone()
    }
    pub fn seal_count(&self) -> usize {
        lock(&self.state).seals
    }
    pub fn prefix(&self) -> PhysicalPrefix {
        lock(&self.state).prefix.clone()
    }
}
impl OutputLog for ScriptedOutputLog {
    fn path(&self) -> &Path {
        &self.path
    }
    fn append(&self, block: AppendBlock) -> BoxFuture<'static, AppendOutcome> {
        let state = self.state.clone();
        async move {
            let step = {
                let mut state = lock(&state);
                state.requests.push(block);
                state.append_steps.pop_front()
            };
            let Some((gate, outcome)) = step else {
                return AppendOutcome::new(
                    lock(&state).prefix.clone(),
                    Some(RecordingError::Unavailable),
                );
            };
            gate.wait().await;
            lock(&state).prefix = outcome.prefix().clone();
            outcome
        }
        .boxed()
    }
    fn seal(&self) -> BoxFuture<'static, SealOutcome> {
        let state = self.state.clone();
        async move {
            let step = {
                let mut state = lock(&state);
                state.seals += 1;
                state.seal_steps.pop_front()
            };
            let Some((gate, outcome)) = step else {
                return SealOutcome::new(
                    lock(&state).prefix.clone(),
                    Some(RecordingError::Unavailable),
                );
            };
            gate.wait().await;
            lock(&state).prefix = outcome.prefix().clone();
            outcome
        }
        .boxed()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    #[test]
    fn canceled_creation_does_not_run_or_start_native_io() {
        let factory = ScriptedOutputLogFactory::default();
        let future = factory.create(RecordingOptions {
            directory: Some(PathBuf::from("never-created")),
            max_file_mib: 1,
        });
        drop(future);
        assert!(factory.requests().is_empty());
        assert!(matches!(
            block_on(factory.create(RecordingOptions {
                directory: None,
                max_file_mib: 1
            })),
            Err(RecordingError::Unavailable)
        ));
    }
    #[test]
    fn gate_keeps_physical_prefix_unpublished_until_operation_completes() {
        let log = ScriptedOutputLog::new(PathBuf::from("memory.log"));
        let gate = Gate::pending();
        log.enqueue_append(
            gate.clone(),
            AppendOutcome::new(PhysicalPrefix::new(1, 3, 0, false, false), None),
        );
        let mut future = log.append(AppendBlock::new(1, 0, Arc::from(&b"abc"[..])).unwrap());
        assert!(future.as_mut().now_or_never().is_none());
        assert_eq!(log.requests().len(), 1);
        assert_eq!(log.prefix().written_bytes(), 0);
        gate.release();
        assert_eq!(block_on(future).prefix().written_bytes(), 3);
        assert_eq!(log.prefix().written_bytes(), 3);
    }
}
