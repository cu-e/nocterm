//! Bounded append persistence driven by the common Store worker.
use crate::{
    StoreError,
    worker::{
        DiskOperation, DiskWork, DriverControl, NormalOwner,
        captured::{self, CapturedPolicy},
    },
};
use futures::{
    StreamExt,
    channel::mpsc::{self, UnboundedReceiver, UnboundedSender},
    future::BoxFuture,
};
use gpui::{AppContext, Context, Entity};
use nocterm_core::persist::WriteGate;
use nocterm_session::recording::{
    AppendBlock, AppendOutcome, MAX_APPEND_BYTES, OutputLog, OutputLogFactory, PhysicalPrefix,
    RecordingError, RecordingOptions,
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
};

const QUEUE_CAPACITY: usize = 128;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppendStatus {
    path: Option<PathBuf>,
    error: Option<RecordingError>,
    finished: bool,
    bytes: u64,
}

impl AppendStatus {
    pub fn new(
        path: Option<PathBuf>,
        error: Option<RecordingError>,
        finished: bool,
        bytes: u64,
    ) -> Self {
        Self {
            path,
            error,
            finished,
            bytes,
        }
    }
    pub fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }
    pub fn error(&self) -> Option<&RecordingError> {
        self.error.as_ref()
    }
    pub fn finished(&self) -> bool {
        self.finished
    }
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

pub struct AppendPersisted {
    admission: AppendAdmission,
    physical: Arc<Mutex<AppendPhysical>>,
    factory: Arc<dyn OutputLogFactory>,
    options: RecordingOptions,
    control: DriverControl,
    status: AppendStatus,
    completion: Option<Box<dyn FnOnce(&AppendStatus) + 'static>>,
}

pub(crate) enum AppendOpKind {
    Create,
    Append,
    Seal,
}

struct AppendAdmission {
    queue: VecDeque<AppendBlock>,
    next_sequence: u64,
    accepted_bytes: u64,
    closed: bool,
    error: Option<RecordingError>,
    wake_tx: UnboundedSender<()>,
    wake_rx: Option<UnboundedReceiver<()>>,
}

pub(crate) struct AppendPhysical {
    sink: Option<Arc<dyn OutputLog>>,
    prefix: PhysicalPrefix,
    first_disk_error: Option<RecordingError>,
    sealed: bool,
    path: Option<PathBuf>,
}

pub struct AppendHandle {
    entity: Entity<AppendPersisted>,
    status: Arc<Mutex<AppendStatus>>,
}

impl Clone for AppendHandle {
    fn clone(&self) -> Self {
        Self {
            entity: self.entity.clone(),
            status: self.status.clone(),
        }
    }
}

impl AppendHandle {
    pub fn output(&self, bytes: &[u8], cx: &mut gpui::App) {
        if bytes.is_empty() {
            return;
        }
        self.entity.update(cx, |this, cx| {
            this.admit_bytes(bytes, cx);
        });
    }

    pub fn stop(&self, cx: &mut gpui::App) {
        self.entity.update(cx, |this, cx| {
            this.close_admission(cx);
        });
    }

    pub fn status(&self) -> AppendStatus {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn entity(&self) -> Entity<AppendPersisted> {
        self.entity.clone()
    }
}

impl AppendPersisted {
    fn create_operation(&self) -> DiskOperation<AppendOutcome> {
        let factory = self.factory.clone();
        let options = self.options.clone();
        let physical = self.physical.clone();
        DiskOperation::new(
            move || {
                Box::pin(async move {
                    let res = factory.create(options).await;
                    match res {
                        Ok(log) => {
                            let mut guard = physical.lock().unwrap_or_else(|e| e.into_inner());
                            guard.path = Some(log.path().to_path_buf());
                            guard.sink = Some(log);
                            Ok(AppendOutcome::new(guard.prefix.clone(), None))
                        }
                        Err(err) => {
                            let mut guard = physical.lock().unwrap_or_else(|e| e.into_inner());
                            guard.first_disk_error = Some(err.clone());
                            guard.sealed = true;
                            Ok(AppendOutcome::new(guard.prefix.clone(), Some(err)))
                        }
                    }
                })
            },
            |_| {},
        )
    }

    fn append_operation(&self, block: AppendBlock) -> DiskOperation<AppendOutcome> {
        let physical = self.physical.clone();
        DiskOperation::new(
            move || {
                Box::pin(async move {
                    let sink = {
                        let guard = physical.lock().unwrap_or_else(|e| e.into_inner());
                        guard.sink.clone()
                    };
                    let Some(sink) = sink else {
                        return Err(StoreError::Closing);
                    };
                    let outcome = sink.append(block).await;
                    let mut guard = physical.lock().unwrap_or_else(|e| e.into_inner());
                    guard.prefix = outcome.prefix().clone();
                    if guard.first_disk_error.is_none() {
                        guard.first_disk_error = outcome.error().cloned();
                    }
                    Ok(outcome)
                })
            },
            |_| {},
        )
    }

    fn seal_operation(&self) -> DiskOperation<AppendOutcome> {
        let physical = self.physical.clone();
        DiskOperation::new(
            move || {
                Box::pin(async move {
                    let sink = {
                        let guard = physical.lock().unwrap_or_else(|e| e.into_inner());
                        guard.sink.clone()
                    };
                    let Some(sink) = sink else {
                        return Err(StoreError::Closing);
                    };
                    let outcome = sink.seal().await;
                    let mut guard = physical.lock().unwrap_or_else(|e| e.into_inner());
                    guard.prefix = outcome.prefix().clone();
                    guard.sealed = true;
                    if guard.first_disk_error.is_none() {
                        guard.first_disk_error = outcome.error().cloned();
                    }
                    Ok(outcome)
                })
            },
            |_| {},
        )
    }
    pub fn new(
        factory: Arc<dyn OutputLogFactory>,
        options: RecordingOptions,
        completion: Option<Box<dyn FnOnce(&AppendStatus) + 'static>>,
        cx: &mut Context<Self>,
    ) -> (Self, AppendHandle) {
        let (wake_tx, wake_rx) = mpsc::unbounded();
        let admission = AppendAdmission {
            queue: VecDeque::with_capacity(QUEUE_CAPACITY),
            next_sequence: 1,
            accepted_bytes: 0,
            closed: false,
            error: None,
            wake_tx,
            wake_rx: Some(wake_rx),
        };
        let physical = Arc::new(Mutex::new(AppendPhysical {
            sink: None,
            prefix: PhysicalPrefix::default(),
            first_disk_error: None,
            sealed: false,
            path: None,
        }));
        let status = AppendStatus::default();
        let shared_status = Arc::new(Mutex::new(status.clone()));

        let mut instance = Self {
            admission,
            physical,
            factory,
            options,
            control: DriverControl::default(),
            status,
            completion,
        };
        crate::worker::start(&mut instance, cx);

        let entity = cx.entity().clone();
        let handle = AppendHandle {
            entity,
            status: shared_status,
        };
        (instance, handle)
    }

    fn admit_bytes(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        if self.admission.closed || self.admission.error.is_some() {
            return;
        }

        let mut start = 0;
        let mut admitted_any = false;
        while start < bytes.len() {
            let mut end = (start + MAX_APPEND_BYTES).min(bytes.len());
            while end < bytes.len() && end > start && (bytes[end] & 0xc0 == 0x80) {
                end -= 1;
            }
            if end == start {
                end = (start + MAX_APPEND_BYTES).min(bytes.len());
            }

            if self.admission.queue.len() >= QUEUE_CAPACITY {
                self.admission.closed = true;
                self.admission.error = Some(RecordingError::Full);
                self.update_status(cx);
                break;
            }

            let slice = &bytes[start..end];
            let expected_offset = self.admission.accepted_bytes;
            let sequence = self.admission.next_sequence;
            let block = match AppendBlock::new(sequence, expected_offset, Arc::from(slice)) {
                Ok(b) => b,
                Err(err) => {
                    self.admission.closed = true;
                    self.admission.error = Some(err);
                    self.update_status(cx);
                    break;
                }
            };

            self.admission.next_sequence += 1;
            self.admission.accepted_bytes += slice.len() as u64;
            self.admission.queue.push_back(block);
            admitted_any = true;
            start = end;
        }

        if admitted_any {
            let _ = self.admission.wake_tx.unbounded_send(());
        }
    }

    pub fn close_admission(&mut self, cx: &mut Context<Self>) {
        if !self.admission.closed {
            self.admission.closed = true;
            let _ = self.admission.wake_tx.unbounded_send(());
            self.update_status(cx);
        }
    }

    fn update_status(&mut self, cx: &mut Context<Self>) {
        let phys = self.physical.lock().unwrap_or_else(|e| e.into_inner());
        self.status.path = phys.path.clone();
        self.status.bytes = phys.prefix.written_bytes();
        self.status.finished = phys.sealed;
        self.status.error = self
            .admission
            .error
            .clone()
            .or_else(|| phys.first_disk_error.clone());
        cx.notify();
    }
}

impl NormalOwner for AppendPersisted {
    type Physical = AppendOutcome;
    type Tag = AppendOpKind;

    fn control(&mut self) -> &mut DriverControl {
        &mut self.control
    }

    fn take_operation(
        &mut self,
        _cx: &mut Context<Self>,
    ) -> Option<DiskWork<Self::Physical, Self::Tag>> {
        let phys_guard = self.physical.lock().unwrap_or_else(|e| e.into_inner());
        if phys_guard.sealed {
            return None;
        }
        drop(phys_guard);

        // 1. Create operation if sink not yet created
        let phys_has_sink = self
            .physical
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .sink
            .is_some();
        if !phys_has_sink {
            return Some(DiskWork::new(
                self.create_operation(),
                AppendOpKind::Create,
                None,
            ));
        }

        // 2. Next append block from admission queue
        if let Some(block) = self.admission.queue.pop_front() {
            return Some(DiskWork::new(
                self.append_operation(block),
                AppendOpKind::Append,
                None,
            ));
        }

        // 3. Seal if admission closed and queue empty
        if self.admission.closed {
            return Some(DiskWork::new(
                self.seal_operation(),
                AppendOpKind::Seal,
                None,
            ));
        }

        None
    }

    fn apply_outcome(
        &mut self,
        _tag: Self::Tag,
        outcome: Result<Self::Physical, StoreError>,
        cx: &mut Context<Self>,
    ) {
        if let Err(err) = outcome {
            let mut phys = self.physical.lock().unwrap_or_else(|e| e.into_inner());
            if phys.first_disk_error.is_none() {
                phys.first_disk_error = Some(RecordingError::CompletionDropped);
            }
            phys.sealed = true;
            let _ = err;
        }

        self.update_status(cx);

        if let (true, Some(completion)) = (self.status.finished, self.completion.take()) {
            completion(&self.status);
        }
    }

    fn wait_for_work(&mut self) -> Option<BoxFuture<'static, ()>> {
        let mut rx = self.admission.wake_rx.take()?;
        Some(Box::pin(async move {
            let _ = rx.next().await;
        }))
    }
}

pub struct CapturedAppend {
    gate: WriteGate,
    factory: Arc<dyn OutputLogFactory>,
    options: RecordingOptions,
    physical: Arc<Mutex<AppendPhysical>>,
    pending: VecDeque<AppendBlock>,
    admission_error: Option<RecordingError>,
}

impl CapturedAppend {
    pub(crate) fn new(
        gate: WriteGate,
        factory: Arc<dyn OutputLogFactory>,
        options: RecordingOptions,
        physical: Arc<Mutex<AppendPhysical>>,
        pending: VecDeque<AppendBlock>,
        admission_error: Option<RecordingError>,
    ) -> Self {
        Self {
            gate,
            factory,
            options,
            physical,
            pending,
            admission_error,
        }
    }

    fn create_operation(&self) -> DiskOperation<AppendOutcome> {
        let factory = self.factory.clone();
        let options = self.options.clone();
        let physical = self.physical.clone();
        DiskOperation::new(
            move || {
                Box::pin(async move {
                    match factory.create(options).await {
                        Ok(log) => {
                            let mut guard = physical.lock().unwrap_or_else(|e| e.into_inner());
                            guard.path = Some(log.path().to_path_buf());
                            guard.sink = Some(log);
                            Ok(AppendOutcome::new(guard.prefix.clone(), None))
                        }
                        Err(err) => {
                            let mut guard = physical.lock().unwrap_or_else(|e| e.into_inner());
                            guard.first_disk_error = Some(err.clone());
                            guard.sealed = true;
                            Ok(AppendOutcome::new(guard.prefix.clone(), Some(err)))
                        }
                    }
                })
            },
            |_| {},
        )
    }

    fn append_operation(&self, block: AppendBlock) -> DiskOperation<AppendOutcome> {
        let physical = self.physical.clone();
        DiskOperation::new(
            move || {
                Box::pin(async move {
                    let sink = {
                        let guard = physical.lock().unwrap_or_else(|e| e.into_inner());
                        guard.sink.clone()
                    };
                    let Some(sink) = sink else {
                        return Err(StoreError::Closing);
                    };
                    let outcome = sink.append(block).await;
                    let mut guard = physical.lock().unwrap_or_else(|e| e.into_inner());
                    guard.prefix = outcome.prefix().clone();
                    if guard.first_disk_error.is_none() {
                        guard.first_disk_error = outcome.error().cloned();
                    }
                    Ok(outcome)
                })
            },
            |_| {},
        )
    }

    fn seal_operation(&self) -> DiskOperation<AppendOutcome> {
        let physical = self.physical.clone();
        DiskOperation::new(
            move || {
                Box::pin(async move {
                    let sink = {
                        let guard = physical.lock().unwrap_or_else(|e| e.into_inner());
                        guard.sink.clone()
                    };
                    let Some(sink) = sink else {
                        return Err(StoreError::Closing);
                    };
                    let outcome = sink.seal().await;
                    let mut guard = physical.lock().unwrap_or_else(|e| e.into_inner());
                    guard.prefix = outcome.prefix().clone();
                    guard.sealed = true;
                    if guard.first_disk_error.is_none() {
                        guard.first_disk_error = outcome.error().cloned();
                    }
                    Ok(outcome)
                })
            },
            |_| {},
        )
    }
}

impl CapturedPolicy for CapturedAppend {
    type Physical = AppendOutcome;
    type Output = AppendStatus;

    fn gate(&self) -> WriteGate {
        self.gate.clone()
    }

    fn joined(&mut self) {}

    fn next_operation(&mut self) -> BoxFuture<'_, Option<DiskOperation<Self::Physical>>> {
        Box::pin(async move {
            let phys_guard = self.physical.lock().unwrap_or_else(|e| e.into_inner());
            if phys_guard.sealed {
                return None;
            }
            drop(phys_guard);

            // 1. Create if needed
            let has_sink = self
                .physical
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .sink
                .is_some();
            if !has_sink {
                return Some(self.create_operation());
            }

            // 2. Append pending blocks
            if let Some(block) = self.pending.pop_front() {
                return Some(self.append_operation(block));
            }

            // 3. Seal
            Some(self.seal_operation())
        })
    }

    fn apply_outcome(&mut self, outcome: Result<Self::Physical, StoreError>) {
        if let Err(err) = outcome {
            let mut guard = self.physical.lock().unwrap_or_else(|e| e.into_inner());
            if guard.first_disk_error.is_none() {
                guard.first_disk_error = Some(RecordingError::CompletionDropped);
            }
            guard.sealed = true;
            let _ = err;
        }
    }

    fn finish(self) -> Self::Output {
        let guard = self.physical.lock().unwrap_or_else(|e| e.into_inner());
        AppendStatus {
            path: guard.path.clone(),
            error: self
                .admission_error
                .or_else(|| guard.first_disk_error.clone()),
            finished: guard.sealed,
            bytes: guard.prefix.written_bytes(),
        }
    }
}

pub struct RecordingRegistry {
    entries: Arc<Mutex<Vec<Entity<AppendPersisted>>>>,
    factory: Arc<dyn OutputLogFactory>,
}

impl RecordingRegistry {
    pub fn new(factory: Arc<dyn OutputLogFactory>) -> Self {
        Self {
            entries: Arc::new(Mutex::new(Vec::new())),
            factory,
        }
    }

    pub fn factory(&self) -> Arc<dyn OutputLogFactory> {
        self.factory.clone()
    }

    pub fn open(&self, options: RecordingOptions, cx: &mut gpui::App) -> AppendHandle {
        let entries = self.entries.clone();
        let entity = cx.new(|cx| {
            let (persisted, _) = AppendPersisted::new(self.factory.clone(), options, None, cx);
            persisted
        });
        entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(entity.clone());
        AppendHandle {
            entity: entity.clone(),
            status: Arc::new(Mutex::new(AppendStatus::default())),
        }
    }

    pub fn register(&self, coordinator: &crate::ShutdownCoordinator) -> Result<(), StoreError> {
        let entries = self.entries.clone();
        coordinator.register_capture(move |cx| {
            let mut list = Vec::new();
            let mut entries_guard = entries.lock().unwrap_or_else(|e| e.into_inner());
            for entity in entries_guard.iter() {
                let captured = entity.update(cx, |this, _| {
                    this.admission.closed = true;
                    this.control.freeze();
                    CapturedAppend::new(
                        this.control.gate(),
                        this.factory.clone(),
                        this.options.clone(),
                        this.physical.clone(),
                        std::mem::take(&mut this.admission.queue),
                        this.admission.error.clone(),
                    )
                });
                list.push(captured);
            }
            entries_guard.clear();
            Box::pin(async move {
                for item in list {
                    captured::run(item).await;
                }
                Ok(())
            })
        })
    }
}

#[cfg(test)]
mod tests;
