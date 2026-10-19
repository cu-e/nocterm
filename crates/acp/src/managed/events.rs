//! Original native events and FIFO fences use the actual prepaid channel owners.
use super::{
    budget::SourceBudget,
    channel::{self, Receiver, Sender},
    error::DecodeError,
};
use nocterm_ai::{
    connection::{
        ManagedAgentEvent, ManagedEventAcknowledgement, ManagedEventBarrier, ManagedEventReceiver,
        ManagedEventSource,
    },
    model::json::deferred::JsonDecodeError,
};
use std::task::{Context, Poll};
type Error = DecodeError<JsonDecodeError>;
impl ManagedEventSource for Receiver<ManagedAgentEvent> {
    fn poll_event(&mut self, cx: &mut Context<'_>) -> Poll<Option<ManagedAgentEvent>> {
        self.poll_recv(cx)
    }
}
pub(super) fn channel(
    budget: &SourceBudget,
) -> Result<(Sender<ManagedAgentEvent>, ManagedEventReceiver), Error> {
    let (sender, receiver) = channel::bounded(budget.clone(), 256)?;
    let receiver = ManagedEventReceiver::new(budget.context(), receiver)?;
    Ok((sender, receiver))
}
struct Acknowledgement {
    sender: Option<Sender<()>>,
}
impl ManagedEventAcknowledgement for Acknowledgement {
    fn acknowledge(&mut self) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.try_send(());
        }
    }
}
/// Acknowledgement and abandonment refer to this exact original one-shot queue.
/// They never look up the current document or substitute a fresh completion.
pub(super) fn fence(budget: &SourceBudget) -> Result<(ManagedEventBarrier, Receiver<()>), Error> {
    let (sender, receiver) = channel::bounded(budget.clone(), 1)?;
    let barrier = ManagedEventBarrier::new(
        budget.context(),
        Acknowledgement {
            sender: Some(sender),
        },
    )?;
    Ok((barrier, receiver))
}
