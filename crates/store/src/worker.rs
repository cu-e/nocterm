//! The one foreground/background drive for ordered persistence policies.
use crate::StoreError;
use futures::{channel::oneshot, future::BoxFuture};
use gpui::{Context, Task};
use nocterm_core::persist::{WriteGate, Writing};
pub(super) mod captured;

type Operation<R> = Box<dyn FnOnce() -> BoxFuture<'static, Result<R, StoreError>> + Send>;
type Recorder<R> = Box<dyn FnOnce(&Result<R, StoreError>) + Send>;
pub(super) struct DiskOperation<R> {
    perform: Operation<R>,
    record: Recorder<R>,
}
impl<R: Send + 'static> DiskOperation<R> {
    pub(super) fn new(
        perform: impl FnOnce() -> BoxFuture<'static, Result<R, StoreError>> + Send + 'static,
        record: impl FnOnce(&Result<R, StoreError>) + Send + 'static,
    ) -> Self {
        Self {
            perform: Box::new(perform),
            record: Box::new(record),
        }
    }
    pub(super) async fn execute(self) -> Result<R, StoreError> {
        let outcome = (self.perform)().await;
        (self.record)(&outcome);
        outcome
    }
}
pub(super) struct DiskWork<R, Tag> {
    operation: DiskOperation<R>,
    tag: Tag,
    writing: Option<Writing>,
}
impl<R, Tag> DiskWork<R, Tag> {
    pub(super) fn new(operation: DiskOperation<R>, tag: Tag, writing: Option<Writing>) -> Self {
        Self {
            operation,
            tag,
            writing,
        }
    }
}
#[derive(Default)]
pub(super) struct DriverControl {
    gate: WriteGate,
    task: Option<Task<()>>,
}
impl DriverControl {
    pub(super) fn gate(&self) -> WriteGate {
        self.gate.clone()
    }
    pub(super) fn is_running(&self) -> bool {
        self.task.is_some()
    }
    pub(super) fn freeze(&mut self) {
        self.gate.freeze();
        self.task.take();
    }
}
/// Only Store policies implement this private ownership protocol.
pub(super) trait NormalOwner: Sized + 'static {
    type Physical: Send + 'static;
    type Tag: Send + 'static;
    fn control(&mut self) -> &mut DriverControl;
    fn take_operation(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<DiskWork<Self::Physical, Self::Tag>>;
    fn apply_outcome(
        &mut self,
        tag: Self::Tag,
        outcome: Result<Self::Physical, StoreError>,
        cx: &mut Context<Self>,
    );
    /// Snapshot/keyed stop when idle. An append policy supplies one coalesced wake.
    fn wait_for_work(&mut self) -> Option<BoxFuture<'static, ()>> {
        None
    }
}
pub(super) fn start<O: NormalOwner>(owner: &mut O, cx: &mut Context<O>) {
    let gate = owner.control().gate();
    owner.control().task = Some(cx.spawn(async move |entity, cx| {
        while !gate.is_frozen() {
            let Ok(work) = entity.update(cx, |owner, cx| owner.take_operation(cx)) else {
                break;
            };
            let Some(DiskWork {
                operation,
                tag,
                writing,
            }) = work
            else {
                let Ok(Some(wake)) = entity.update(cx, |owner, _| owner.wait_for_work()) else {
                    break;
                };
                wake.await;
                continue;
            };
            let disk_gate = gate.clone();
            let (sender, result) = oneshot::channel();
            cx.background_executor()
                .spawn(async move {
                    let _writing = writing;
                    let outcome = match disk_gate.normal().await {
                        Some(_guard) => operation.execute().await,
                        None => Err(StoreError::Closing),
                    };
                    let _ = sender.send(outcome);
                })
                .detach();
            let outcome = result.await.unwrap_or(Err(StoreError::CompletionDropped));
            if gate.is_frozen() {
                break;
            }
            if entity
                .update(cx, |owner, cx| owner.apply_outcome(tag, outcome, cx))
                .is_err()
            {
                break;
            }
        }
        let _ = entity.update(cx, |owner, _| owner.control().task = None);
    }));
}
