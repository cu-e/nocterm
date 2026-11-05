//! Original-account command flows and the sealed bytes they retain until settlement.
use super::{
    account::DeviceAccount,
    checkpoints::AccountPlace,
    driver::{self, Effects},
    effects::Event,
    mapping,
    outbox::{Outbox, OutboxError},
};
use nocterm_cloud_conversation_model::CloudConversationError;
use nocterm_cloud_device::{
    command::{Deliver, DeliveryUpdate},
    context::ChatRef,
    flow::{Effect, EffectId, Flow, Step},
    open::OpenChat,
    transcript::CommandState,
};
use nocterm_core::error::SharedCause;
use std::sync::Arc;

pub(super) struct Deliveries {
    flows: Vec<(u32, Deliver)>,
    kept: Vec<(u32, [u8; 16])>,
    account: Arc<DeviceAccount>,
    outbox: Arc<Outbox>,
    place: AccountPlace,
    chat: ChatRef,
    failure: Option<CloudConversationError>,
}
impl Deliveries {
    pub(super) fn new(
        account: Arc<DeviceAccount>,
        outbox: Arc<Outbox>,
        place: AccountPlace,
        chat: ChatRef,
    ) -> Self {
        Self {
            flows: Vec::new(),
            kept: Vec::new(),
            account,
            outbox,
            place,
            chat,
            failure: None,
        }
    }
    pub(super) fn len(&self) -> usize {
        self.flows.len()
    }
    pub(super) fn take_failure(&mut self) -> Option<CloudConversationError> {
        self.failure.take()
    }
    pub(super) fn is_empty(&self) -> bool {
        self.flows.is_empty()
    }
    pub(super) fn insert(&mut self, slot: u32, flow: Deliver, stored: bool) {
        if stored {
            self.kept.push((slot, flow.command_id()));
        }
        self.flows.push((slot, flow));
    }
    pub(super) fn resolve(&mut self, slot: u32, id: EffectId, event: Event) {
        if let Some((_, flow)) = self.flows.iter_mut().find(|(known, _)| *known == slot) {
            driver::answer(flow, id, event);
        }
    }
    /// Decoding and forgetting files run on bounded storage workers; a flow is
    /// returned only under its original account and its original command bytes.
    pub(super) async fn restore(
        &mut self,
        chat: &mut OpenChat,
        wake: Option<String>,
    ) -> Result<Vec<Deliver>, CloudConversationError> {
        let left = self
            .outbox
            .left_async(self.place.clone(), self.chat)
            .await
            .map_err(storage)?;
        self.current()?;
        let mut flows = Vec::new();
        for (command, stored) in left {
            match chat.resume(&stored, wake.clone(), driver::now_ms()) {
                Ok(Some(flow)) => flows.push(flow),
                Err(
                    error @ (nocterm_cloud_device::CloudDeviceError::LargePublicationUnsupported
                    | nocterm_cloud_device::CloudDeviceError::Capacity),
                ) => {
                    // A valid original command may outlive a hub capability downgrade. Keep
                    // its durable bytes until that hub can publish them again.
                    return Err(super::mapping::error(error));
                }
                result => {
                    if let Err(error) = result {
                        tracing::debug!(error = %nocterm_core::error::chain(&error), "a kept Cloud command was not resumed");
                    }
                    self.outbox
                        .settled_async(self.place.clone(), self.chat, command)
                        .await
                        .map_err(storage)?;
                }
            }
        }
        Ok(flows)
    }
    /// Progresses each delivery once. Publication never bypasses a failed native
    /// keep, and network effects start only after its physical write completed.
    pub(super) async fn pump(
        &mut self,
        chat: &mut OpenChat,
        effects: &mut Effects<u32>,
    ) -> Result<(bool, Option<i64>), CloudConversationError> {
        let (mut progressed, mut until) = (false, None::<i64>);
        let mut index = 0;
        while index < self.flows.len() {
            let (slot, flow) = &mut self.flows[index];
            let slot = *slot;
            match flow.next(driver::now_ms()) {
                Step::Effect(id, effect) => {
                    let keep = matches!(effect, Effect::Publish(_))
                        && !self.kept.iter().any(|(known, _)| *known == slot);
                    let stored = keep.then(|| (flow.command_id(), chat.stored(flow)));
                    if let Some((command, stored)) = stored {
                        let stored = stored.map_err(mapping::error)?.ok_or_else(|| {
                            mapping::error(nocterm_cloud_device::CloudDeviceError::Protocol)
                        })?;
                        if let Err(error) = self
                            .outbox
                            .keep_async(self.place.clone(), self.chat, command, stored)
                            .await
                        {
                            self.unpublished(chat, command, error).await;
                            self.flows.remove(index);
                            progressed = true;
                            continue;
                        }
                        self.kept.push((slot, command));
                    }
                    self.current()?;
                    effects.start(slot, id, effect);
                    progressed = true;
                }
                Step::Output(update) => {
                    chat.delivery(&update);
                    progressed = true;
                }
                Step::Idle { until_ms } => {
                    until = [until, until_ms].into_iter().flatten().min();
                    index += 1;
                }
                Step::Done => {
                    self.flows.remove(index);
                    self.forget(slot).await?;
                    progressed = true;
                }
                Step::Failed(error) => {
                    tracing::warn!(error = %nocterm_core::error::chain(&error), "a Cloud command failed");
                    chat.delivery(&DeliveryUpdate {
                        command_id: flow.command_id(),
                        state: CommandState::Refused,
                        cursor: None,
                    });
                    self.flows.remove(index);
                    self.forget(slot).await?;
                    progressed = true;
                }
            }
        }
        Ok((progressed, until))
    }
    /// No network publication was started. A failed atomic publication may still
    /// have installed its file, so confirm removal before calling it refused.
    async fn unpublished(&mut self, chat: &mut OpenChat, command: [u8; 16], error: OutboxError) {
        tracing::warn!(error = %nocterm_core::error::chain(&error), "a Cloud command was not published because its native keep failed");
        let state = match self
            .outbox
            .settled_async(self.place.clone(), self.chat, command)
            .await
        {
            Ok(()) => CommandState::Refused,
            Err(cleanup) => {
                tracing::warn!(error = %nocterm_core::error::chain(&cleanup), "an unpublished Cloud command may remain in the outbox");
                CommandState::Unreachable
            }
        };
        chat.delivery(&DeliveryUpdate {
            command_id: command,
            state,
            cursor: None,
        });
        self.failure = Some(storage(error));
    }
    fn current(&self) -> Result<(), CloudConversationError> {
        self.account
            .check()
            .map_err(|_| CloudConversationError::Stale)
    }
    async fn forget(&mut self, slot: u32) -> Result<(), CloudConversationError> {
        if let Some(index) = self.kept.iter().position(|(known, _)| *known == slot) {
            let (_, command) = self.kept[index];
            self.outbox
                .settled_async(self.place.clone(), self.chat, command)
                .await
                .map_err(storage)?;
            self.kept.remove(index);
        }
        Ok(())
    }
}
fn storage(error: OutboxError) -> CloudConversationError {
    CloudConversationError::Storage(SharedCause::new(error))
}
