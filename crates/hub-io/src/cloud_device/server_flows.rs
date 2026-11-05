//! The SSH flows of one open chat (ADR 0139): binding the chat to servers, and answering the
//! runner's certificate requests while the account is unlocked.
//!
//! The core decides what may be signed: `SshAnswer` certifies only a request the verified
//! history holds, inside a binding this account's current CA signed and that was not revoked,
//! refused or expired, for servers still trusted. A refused answer is not retried here; the
//! runner asks again with a new request.
use super::driver::{self, Effects};
use super::effects::Event;
use super::mapping;
use futures::channel::oneshot;
use nocterm_cloud_conversation_model::CloudConversationError;
use nocterm_cloud_device::flow::{EffectId, Flow, Step};
use nocterm_cloud_device::open::OpenChat;
use nocterm_cloud_device::ssh::{BindServers, SshAnswer, SshUpdate};

/// Told once the binding is signed and on its way, or why it was not.
pub(super) type BindReply = oneshot::Sender<Result<(), CloudConversationError>>;

enum ServerFlow {
    Bind(BindServers, Option<BindReply>),
    Answer(SshAnswer),
}
impl ServerFlow {
    fn next(&mut self, now_ms: i64) -> Step<SshUpdate> {
        match self {
            Self::Bind(flow, _) => flow.next(now_ms),
            Self::Answer(flow) => flow.next(now_ms),
        }
    }
    fn answer(&mut self, id: EffectId, event: Event) {
        match self {
            Self::Bind(flow, _) => driver::answer(flow, id, event),
            Self::Answer(flow) => driver::answer(flow, id, event),
        }
    }
    fn reply(&mut self, result: Result<(), CloudConversationError>) {
        if let Self::Bind(_, reply) = self
            && let Some(reply) = reply.take()
        {
            let _ = reply.send(result);
        }
    }
}

/// The SSH flows in progress, each under the effect slot it was started with.
#[derive(Default)]
pub(super) struct ServerFlows {
    flows: Vec<(u32, ServerFlow)>,
}
impl ServerFlows {
    pub(super) fn bind(&mut self, slot: u32, flow: BindServers, reply: BindReply) {
        self.flows.push((slot, ServerFlow::Bind(flow, Some(reply))));
    }
    /// Answers `request` of `chat` under `slot` when the core allows it; a refusal (outside
    /// the binding, superseded, revoked) leaves the request to the runner's next one.
    pub(super) fn answer(
        &mut self,
        chat: &mut OpenChat,
        slot: u32,
        request: &[u8; 16],
        now_ms: i64,
    ) {
        match chat.answer_ssh(request, now_ms) {
            Ok(flow) => self.flows.push((slot, ServerFlow::Answer(flow))),
            Err(error) => {
                tracing::debug!(error = %nocterm_core::error::chain(&error), "a Cloud chat's SSH certificate request was not answered");
            }
        }
    }
    pub(super) fn is_empty(&self) -> bool {
        self.flows.is_empty()
    }
    /// Whether an SSH flow runs under `slot`.
    pub(super) fn owns(&self, slot: u32) -> bool {
        self.flows.iter().any(|(known, _)| *known == slot)
    }
    /// Hands `event` to the flow of `slot`, if it still runs.
    pub(super) fn resolve(&mut self, slot: u32, id: EffectId, event: Event) {
        if let Some((_, flow)) = self.flows.iter_mut().find(|(known, _)| *known == slot) {
            flow.answer(id, event);
        }
    }
    /// Runs every flow until each waits; a delivery's outcome goes to `chat`. Returns whether
    /// anything moved and the earliest time a flow wants to be called again.
    pub(super) fn pump(
        &mut self,
        chat: &mut OpenChat,
        effects: &mut Effects<u32>,
        now_ms: i64,
    ) -> (bool, Option<i64>) {
        let (mut progressed, mut until) = (false, None::<i64>);
        let mut index = 0;
        while index < self.flows.len() {
            let (slot, flow) = &mut self.flows[index];
            let slot = *slot;
            match flow.next(now_ms) {
                Step::Effect(id, effect) => {
                    effects.start(slot, id, effect);
                    progressed = true;
                }
                Step::Output(update) => {
                    match update {
                        SshUpdate::Bound { .. } => flow.reply(Ok(())),
                        SshUpdate::Certified { .. } => {
                            tracing::debug!("a Cloud chat's SSH certificate request was answered");
                        }
                        SshUpdate::Delivery(delivery) => chat.delivery(&delivery),
                    }
                    progressed = true;
                }
                Step::Idle { until_ms } => {
                    until = [until, until_ms].into_iter().flatten().min();
                    index += 1;
                }
                Step::Done => {
                    flow.reply(Err(CloudConversationError::Ended));
                    self.flows.remove(index);
                    progressed = true;
                }
                Step::Failed(error) => {
                    tracing::debug!(error = %nocterm_core::error::chain(&error), "a Cloud chat SSH flow ended");
                    flow.reply(Err(mapping::error(error)));
                    self.flows.remove(index);
                    progressed = true;
                }
            }
        }
        (progressed, until)
    }
}
