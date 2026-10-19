//! A connection's event pump releases its native receiver before its close is reported.
use futures::{
    channel::oneshot,
    future::{Either, select},
};
use gpui::Task;
use std::pin::pin;

/// The pump task's halves of the stop and completion signals.
pub(super) struct PumpSignals {
    stop: oneshot::Receiver<()>,
    done: oneshot::Sender<()>,
}
/// The owner's halves; dropping them also stops the pump.
pub(super) struct PumpControl {
    stop: oneshot::Sender<()>,
    done: oneshot::Receiver<()>,
}
pub(super) fn signals() -> (PumpSignals, PumpControl) {
    let (stop, stop_rx) = oneshot::channel();
    let (done_tx, done) = oneshot::channel();
    (
        PumpSignals {
            stop: stop_rx,
            done: done_tx,
        },
        PumpControl { stop, done },
    )
}
impl PumpSignals {
    /// Whether `next` produced a value before the owner asked the pump to stop.
    pub(super) async fn until_stopped<T>(
        &mut self,
        next: impl std::future::Future<Output = Option<T>>,
    ) -> Option<T> {
        match select(pin!(next), &mut self.stop).await {
            Either::Left((value, _)) => value,
            Either::Right(_) => None,
        }
    }
    /// Called after the pump dropped everything it owned.
    pub(super) fn finished(self) {
        let _ = self.done.send(());
    }
}
pub(super) struct EventPump {
    _task: Task<()>,
    control: PumpControl,
}
impl EventPump {
    pub(super) fn new(task: Task<()>, control: PumpControl) -> Self {
        Self {
            _task: task,
            control,
        }
    }
    /// Resolves once the pump has dropped its event receiver.
    pub(super) async fn stop(self) {
        let Self { _task, control } = self;
        let PumpControl { stop, done } = control;
        let _ = stop.send(());
        let _ = done.await;
    }
}
