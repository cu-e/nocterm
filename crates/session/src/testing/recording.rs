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

    #[test]
    fn recording_categories_keep_native_sources_and_existing_equality() {
        use std::error::Error;
        for error in [
            RecordingError::create(
                "memory.log",
                std::io::Error::from(std::io::ErrorKind::StorageFull),
            ),
            RecordingError::append(
                "memory.log",
                7,
                std::io::Error::from(std::io::ErrorKind::StorageFull),
            ),
            RecordingError::flush(
                "memory.log",
                std::io::Error::from(std::io::ErrorKind::StorageFull),
            ),
            RecordingError::sync(
                "memory.log",
                std::io::Error::from(std::io::ErrorKind::StorageFull),
            ),
        ] {
            let clone = error.clone();
            let native = error
                .source()
                .unwrap()
                .source()
                .unwrap()
                .downcast_ref::<std::io::Error>()
                .unwrap();
            let shared = clone
                .source()
                .unwrap()
                .source()
                .unwrap()
                .downcast_ref::<std::io::Error>()
                .unwrap();
            assert_eq!(native.kind(), std::io::ErrorKind::StorageFull);
            assert!(std::ptr::eq(native, shared));
            assert_eq!(error, clone);
            assert_eq!(error.to_string(), clone.to_string());
        }
        assert_eq!(
            RecordingError::append("memory.log", 7, std::io::ErrorKind::Other.into()),
            RecordingError::append("memory.log", 7, std::io::ErrorKind::BrokenPipe.into())
        );
        assert_ne!(
            RecordingError::append("memory.log", 7, std::io::ErrorKind::Other.into()),
            RecordingError::append("memory.log", 8, std::io::ErrorKind::Other.into())
        );
        assert_ne!(
            RecordingError::flush("memory.log", std::io::ErrorKind::Other.into()),
            RecordingError::sync("memory.log", std::io::ErrorKind::Other.into())
        );
    }
    #[test]
    fn held_scripted_append_and_seal_keep_error_identity_with_physical_prefix() {
        use std::error::Error;
        let log = ScriptedOutputLog::new(PathBuf::from("memory.log"));
        let error = RecordingError::append("memory.log", 2, std::io::ErrorKind::StorageFull.into());
        let original = error.clone();
        let gate = Gate::pending();
        log.enqueue_append(
            gate.clone(),
            AppendOutcome::new(
                PhysicalPrefix::new(0, 2, 2, false, false),
                Some(error.clone()),
            ),
        );
        let mut append = log.append(AppendBlock::new(1, 0, Arc::from(&b"bytes"[..])).unwrap());
        assert!(append.as_mut().now_or_never().is_none());
        assert_eq!(log.prefix().written_bytes(), 0);
        gate.release();
        let result = block_on(append);
        assert_eq!(result.prefix().partial_bytes(), 2);
        log.enqueue_seal(
            Gate::open(),
            AppendOutcome::new(PhysicalPrefix::new(0, 2, 2, false, true), Some(error)),
        );
        let sealed = block_on(log.seal());
        assert!(sealed.prefix().closed());
        assert!(!sealed.prefix().synced());
        assert_eq!(log.seal_count(), 1);
        let native = original
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap();
        for observed in [result.error().unwrap(), sealed.error().unwrap()] {
            let shared = observed
                .source()
                .unwrap()
                .source()
                .unwrap()
                .downcast_ref::<std::io::Error>()
                .unwrap();
            assert!(std::ptr::eq(native, shared));
        }
    }
}
