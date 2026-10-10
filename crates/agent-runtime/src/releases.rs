//! Lease releases remain owned by the Runtime service rather than by a view.
use super::{Runtime, lease::ReleasedLease};
use gpui::{Context, Task};
pub(super) struct LeaseInbox {
    sender: async_channel::Sender<ReleasedLease>,
    receiver: async_channel::Receiver<ReleasedLease>,
    task: Option<Task<()>>,
}
impl LeaseInbox {
    pub(super) fn new(cx: &mut Context<Runtime>) -> Self {
        let (sender, receiver) = async_channel::unbounded();
        let pending = receiver.clone();
        let task = cx.spawn(async move |runtime, cx| {
            while let Ok(released) = pending.recv().await {
                if runtime
                    .update(cx, |runtime, cx| runtime.release_lease(released, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            sender,
            receiver,
            task: Some(task),
        }
    }
    pub(super) fn sender(&self) -> async_channel::Sender<ReleasedLease> {
        self.sender.clone()
    }
    pub(super) fn drain(&self) -> Vec<ReleasedLease> {
        let mut released = Vec::new();
        while let Ok(lease) = self.receiver.try_recv() {
            released.push(lease)
        }
        released
    }
    pub(super) fn stop(&mut self) {
        self.task.take();
        self.sender.close();
    }
}
