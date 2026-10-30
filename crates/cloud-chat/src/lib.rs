//! Desktop Cloud conversation projection and captured command delivery, without views.
//!
//! Owns: one installation's bounded conversation registry, independent window
//! observers, GPUI replay, immutable accepted transcript, pending command IDs and
//! original-source publication. Uses the existing Local installation history
//! admission, including retained pages, pending commands and cache work.
//! Does not own: login/unlock, cryptography, native storage/network, runner execution,
//! wire encodings, model/tool credentials, views or Local SessionLease lifecycle.
//! Ports used: cloud-model::projection::ports::CapturedConversationSource and
//! agent-runtime::history_admission. Ports provided: agent-chat::ConversationSource.
//! Settings sections: none. Globals: none.
//! Invariants: source is captured before enqueue and rechecked after every await;
//! atomic publication is delegated to the original native membership. Accepted
//! prompts enter only through verified PromptAccepted. A retry retains its ID and
//! durable body. Captured tool-card choices preserve the exact intent/args/run,
//! observed source/control and cutoff through the same durable command owner.
//! Viewer close never sends Stop. Native ports have no fake default.
mod admission;
mod command_operation;
mod conversation;
mod error;
mod installation;
mod pending;
mod replay;
pub use conversation::CloudConversation;
pub use error::CloudChatError;
pub use installation::CloudChatInstallation;
pub use pending::{PendingCommandState, PendingCommandView};

use admission::ProjectionResidency;
use command_operation::CommandOperation;
use futures::channel::oneshot;
use gpui::{App, AppContext as _, Context, Entity, Task};
use nocterm_agent_chat::conversation::ConversationError;
use nocterm_agent_runtime::history_admission::HistoryAdmissionHandle;
use nocterm_ai::conversation::{ConversationAction, ConversationPhase};
use nocterm_cloud_model::{
    identity::CommandId,
    projection::{
        ConversationIntent, ConversationProjection, MAX_REPLAY_BYTES, ProjectionBatch,
        ProjectionCursor, ProjectionError,
        ports::{CapturedConversationSource, ConversationCoordinates, ConversationWork},
    },
};
use nocterm_core::allocation::{Allocated, AllocationLease};
use pending::PendingCommands;
use std::{rc::Rc, sync::Arc};

pub struct CloudChat {
    source: Arc<dyn CapturedConversationSource>,
    projection: Option<Arc<Allocated<ConversationProjection>>>,
    empty: ConversationProjection,
    residency: ProjectionResidency,
    pending: PendingCommands,
    subscription: Option<Task<()>>,
    error: Option<CloudChatError>,
    cache: Option<Task<()>>,
    membership: Option<Task<()>>,
}
impl CloudChat {
    /// Every partition receives the same handle from RuntimeHandle::history_admission.
    /// Construction does not create a new budget or an execution/session lease.
    pub fn open(
        source: Arc<dyn CapturedConversationSource>,
        admission: HistoryAdmissionHandle,
        cx: &mut App,
    ) -> Result<Entity<Self>, CloudChatError> {
        source.check_current()?;
        let payload = admission.reserve_payload_reclaiming(None, cx)?;
        let initial = admission.reserve_unowned_bytes(1024, cx)?;
        let empty = ConversationProjection::new(source.binding().clone());
        let allocation =
            AllocationLease::new(initial.bytes(), 1, Arc::new((initial, payload.clone())));
        let projection = Arc::new(Allocated::new(
            ConversationProjection::new(source.binding().clone()),
            allocation,
        ));
        let entity = cx.new(|_| Self {
            source,
            projection: Some(projection),
            empty,
            residency: ProjectionResidency::new(admission, payload),
            pending: PendingCommands::default(),
            subscription: None,
            error: None,
            cache: None,
            membership: None,
        });
        entity.update(cx, |chat, cx| {
            chat.residency.register(cx)?;
            let source = chat.source.clone();
            chat.membership = Some(cx.spawn(async move |chat, cx| {
                source.ended().await;
                let _ = chat.update(cx, |chat, cx| chat.fail(ProjectionError::Stale.into(), cx));
            }));
            chat.resume(cx)
        })?;
        Ok(entity)
    }
    pub fn projection(&self) -> &ConversationProjection {
        self.projection
            .as_ref()
            .map(|value| value.value())
            .unwrap_or(&self.empty)
    }
    pub fn last_error(&self) -> Option<&CloudChatError> {
        self.error.as_ref()
    }
    pub fn pending_commands(&self) -> Vec<PendingCommandView> {
        self.pending.views()
    }
    pub fn is_shown(&self) -> bool {
        self.residency.shown()
    }
    pub fn conversation(
        entity: &Entity<Self>,
    ) -> Rc<dyn nocterm_agent_chat::conversation::ConversationSource> {
        Rc::new(CloudConversation::new(entity))
    }
    pub fn check_current(&self) -> Result<(), CloudChatError> {
        self.source.check_current().map_err(CloudChatError::from)
    }
    pub fn resume(&mut self, cx: &mut Context<Self>) -> Result<(), CloudChatError> {
        self.source.check_current()?;
        if self.subscription.is_none() {
            if self.projection.is_none() {
                let (allocation, payload) = self.residency.allocate(1024, cx)?;
                let projection = ConversationProjection::new(self.source.binding().clone());
                self.projection = Some(Arc::new(Allocated::new(projection, allocation)));
                self.residency.published(payload);
            }
            self.subscription =
                Some(replay::ReplaySubscription::new(self.source.clone()).start(cx));
        }
        Ok(())
    }
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) -> Result<(), CloudChatError> {
        self.source.check_current()?;
        self.residency.set_shown(shown);
        if shown {
            self.resume(cx)?;
        } else {
            self.subscription.take();
        }
        cx.notify();
        Ok(())
    }
    pub fn retry(
        &mut self,
        id: CommandId,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), ConversationError>> {
        if let Err(error) = self.source.check_current() {
            return Task::ready(Err(CloudChatError::from(error).into_conversation()));
        }
        let work = match self.pending.item(id).and_then(|command| {
            self.source
                .allocation_requirements(ConversationWork::Stage {
                    command: command.value(),
                })
        }) {
            Ok(required) => match self
                .pending
                .item(id)
                .map(|command| matches!(command.value().action, ConversationIntent::Stop))
                .map_err(CloudChatError::from)
                .and_then(|urgent| {
                    if urgent {
                        self.residency.urgent(required, cx)
                    } else {
                        self.residency.work(required, cx)
                    }
                    .map_err(CloudChatError::from)
                }) {
                Ok(work) => work,
                Err(error) => {
                    return Task::ready(Err(CloudChatError::from(error).into_conversation()));
                }
            },
            Err(error) => return Task::ready(Err(CloudChatError::from(error).into_conversation())),
        };
        let (waiter, receive) = oneshot::channel();
        let retained = match self.pending.retry(id, waiter) {
            Ok(retained) => retained,
            Err(error) => return Task::ready(Err(CloudChatError::from(error).into_conversation())),
        };
        let task = CommandOperation::new(self.source.clone(), retained, work, true).start(cx);
        self.pending.attach(id, task);
        Self::wait_for_command(receive, cx)
    }
    fn coordinates(&self) -> ConversationCoordinates {
        ConversationCoordinates {
            binding: self.projection().binding().clone(),
            run: self.projection().run().cloned(),
            control: self.projection().control(),
        }
    }
    fn perform(
        &mut self,
        coordinates: ConversationCoordinates,
        action: ConversationAction,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), ConversationError>> {
        match self.submit(coordinates, action, cx) {
            Ok((_, completion)) => completion,
            Err(error) => Task::ready(Err(error)),
        }
    }
    fn submit(
        &mut self,
        coordinates: ConversationCoordinates,
        action: ConversationAction,
        cx: &mut Context<Self>,
    ) -> Result<(CommandId, Task<Result<(), ConversationError>>), ConversationError> {
        let (waiter, receive) = oneshot::channel();
        let id = self
            .enqueue(coordinates, action, waiter, cx)
            .map_err(CloudChatError::into_conversation)?;
        Ok((id, Self::wait_for_command(receive, cx)))
    }
    fn perform_approval(
        &mut self,
        key: nocterm_cloud_model::projection::ApprovalKey,
        control: nocterm_cloud_model::identity::ChatControlRevision,
        choice: nocterm_cloud_model::coordinator::ApprovalChoice,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), ConversationError>> {
        let (waiter, receive) = oneshot::channel();
        let result = self.enqueue_approval(key, control, choice, waiter, cx);
        if let Err(error) = result {
            return Task::ready(Err(error.into_conversation()));
        }
        Self::wait_for_command(receive, cx)
    }
    fn enqueue_approval(
        &mut self,
        key: nocterm_cloud_model::projection::ApprovalKey,
        control: nocterm_cloud_model::identity::ChatControlRevision,
        choice: nocterm_cloud_model::coordinator::ApprovalChoice,
        waiter: pending::CommandWaiter,
        cx: &mut Context<Self>,
    ) -> Result<(), CloudChatError> {
        self.source.check_current()?;
        if self.projection().control() != control
            || self.projection().phase() == ConversationPhase::Deleted
        {
            return Err(ProjectionError::Stale.into());
        }
        self.pending.require_room()?;
        if self.pending.has_approval(key) {
            return Err(ProjectionError::Unavailable.into());
        }
        // Reserve bounded immutable command coordinates BEFORE cloning the
        // observed execution descriptor. Exact call/display payload stays borrowed.
        let allocation = self.residency.allocate(4096 + 512, cx)?.0;
        let card = self
            .projection()
            .approval(key)
            .ok_or(ProjectionError::Unavailable)?;
        let action = ConversationIntent::AnswerApproval(Box::new(card.request(choice)?));
        let coordinates = ConversationCoordinates {
            binding: self.projection().binding().clone(),
            run: Some(card.intent().authority().run().clone()),
            control,
        };
        self.enqueue_intent(coordinates, action, allocation, waiter, cx)
            .map(|_| ())
    }
    fn wait_for_command(
        receive: oneshot::Receiver<Result<(), ProjectionError>>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), ConversationError>> {
        cx.spawn(async move |_, _| {
            receive
                .await
                .unwrap_or(Err(ProjectionError::Stale))
                .map_err(|error| CloudChatError::from(error).into_conversation())
        })
    }
    fn enqueue(
        &mut self,
        coordinates: ConversationCoordinates,
        action: ConversationAction,
        waiter: pending::CommandWaiter,
        cx: &mut Context<Self>,
    ) -> Result<CommandId, CloudChatError> {
        self.source.check_current()?;
        if coordinates != self.coordinates() {
            return Err(ProjectionError::Stale.into());
        }
        if self.projection().phase() == ConversationPhase::Deleted {
            return Err(ProjectionError::Unavailable.into());
        }
        self.pending.require_room()?;
        let bytes =
            nocterm_cloud_model::projection::ports::ConversationCommand::action_bytes(&action);
        if bytes > MAX_REPLAY_BYTES {
            return Err(ProjectionError::Capacity.into());
        }
        let urgent = matches!(action, ConversationAction::Stop);
        let allocation = if urgent {
            self.residency.urgent(
                nocterm_cloud_model::projection::ports::AllocationRequirements::new(
                    bytes.saturating_add(512),
                    1,
                ),
                cx,
            )?
        } else {
            self.residency.allocate(bytes.saturating_add(512), cx)?.0
        };
        self.enqueue_intent(coordinates, action.into(), allocation, waiter, cx)
    }
    fn enqueue_intent(
        &mut self,
        coordinates: ConversationCoordinates,
        action: ConversationIntent,
        allocation: AllocationLease,
        waiter: pending::CommandWaiter,
        cx: &mut Context<Self>,
    ) -> Result<CommandId, CloudChatError> {
        self.source.check_current()?;
        let bytes = action.resident_bytes();
        let urgent = matches!(action, ConversationIntent::Stop);
        let command = self.source.prepare(coordinates.clone(), action.clone())?;
        if command.coordinates != coordinates || command.action != action {
            return Err(ProjectionError::WrongSource.into());
        }
        let required = self
            .source
            .allocation_requirements(ConversationWork::Stage { command: &command })?;
        let work = if urgent {
            if bytes.saturating_add(512).saturating_add(required.bytes())
                > nocterm_agent_runtime::history_admission::URGENT_HISTORY_BYTES
                || required.payloads() > 1
            {
                return Err(ProjectionError::Capacity.into());
            }
            self.residency.urgent(required, cx)?
        } else {
            self.residency.work(required, cx)?
        };
        let id = command.id;
        let source = self.source.clone();
        let mut prepared = Some((command, allocation, waiter));
        let mut inserted = Ok(());
        source.with_current(&mut || {
            if let Some((command, allocation, waiter)) = prepared.take() {
                inserted = self.pending.insert(command, allocation, waiter);
            }
        })?;
        inserted?;
        let retained = self.pending.item(id)?;
        let operation = CommandOperation::new(source, retained, work, false).start(cx);
        self.pending.attach(id, operation);
        cx.notify();
        Ok(id)
    }
    fn finish_command(
        &mut self,
        id: CommandId,
        result: Result<PendingCommandState, ProjectionError>,
        cx: &mut Context<Self>,
    ) {
        self.pending.finish(id, result);
        cx.notify();
    }
    fn record_error(&mut self, error: CloudChatError) {
        self.error = Some(error);
    }
    fn fail(&mut self, error: CloudChatError, cx: &mut Context<Self>) {
        if matches!(error, CloudChatError::Source(ProjectionError::Stale)) {
            self.pending.invalidate();
            self.cache.take();
            self.projection = None;
            self.residency.evicted();
        }
        self.error = Some(error);
        self.subscription.take();
        cx.notify();
    }
    fn can_evict(&self) -> bool {
        !self.is_shown() && self.pending.is_empty() && self.cache.is_none()
    }
    fn evict(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.can_evict() {
            return false;
        }
        self.subscription.take();
        self.projection = None;
        self.residency.evicted();
        cx.notify();
        true
    }
    fn publish(
        &mut self,
        next: Arc<Allocated<ConversationProjection>>,
        accepted: &[CommandId],
    ) -> Result<(), ProjectionError> {
        let source = self.source.clone();
        source.with_current(&mut || {
            self.projection = Some(next.clone());
            self.pending.accepted(accepted);
            self.error = None;
        })
    }
    fn prepare_replay(
        &self,
        cx: &mut Context<Self>,
    ) -> Result<(ProjectionCursor, AllocationLease), CloudChatError> {
        self.source.check_current()?;
        // A native page is a separately retained allocation. Its proof travels
        // into actual workers/results, even after this GPUI observer disappears.
        let required = self
            .source
            .allocation_requirements(ConversationWork::Replay {
                after: self.projection().cursor(),
                maximum_bytes: MAX_REPLAY_BYTES,
            })?;
        let allocation = self.residency.work(required, cx)?;
        Ok((self.projection().cursor(), allocation))
    }
    fn accept_page(
        &mut self,
        batch: Allocated<ProjectionBatch>,
        cx: &mut Context<Self>,
    ) -> Result<(), CloudChatError> {
        self.source.check_current()?;
        let bytes = self
            .projection()
            .resident_bytes()
            .saturating_add(batch.value().projection_growth_bytes()?)
            .saturating_add(4096);
        let (allocation, payload) = self.residency.allocate(bytes, cx)?;
        let (next, accepted) = self.projection().advanced(batch.value())?;
        if next.resident_bytes() > allocation.bytes() {
            return Err(ProjectionError::Capacity.into());
        }
        self.publish(Arc::new(Allocated::new(next, allocation)), &accepted)?;
        self.residency.published(payload);
        cx.notify();
        self.persist_projection(cx);
        Ok(())
    }
    fn persist_projection(&mut self, cx: &mut Context<Self>) {
        if self.cache.is_some() {
            return;
        }
        let source = self.source.clone();
        let Some(projection) = self.projection.clone() else {
            return;
        };
        let required = match source.allocation_requirements(ConversationWork::Cache {
            projection: projection.value(),
        }) {
            Ok(required) => required,
            Err(error) => {
                self.record_error(error.into());
                return;
            }
        };
        let work = match self.residency.work(required, cx) {
            Ok(work) => work,
            Err(error) => {
                self.record_error(error.into());
                return;
            }
        };
        let cursor = projection.value().cursor();
        self.cache = Some(cx.spawn(async move |chat, cx| {
            let result = match source.check_current() {
                Ok(()) => source
                    .cache(projection, work)
                    .await
                    .and_then(|()| source.check_current()),
                Err(error) => Err(error),
            };
            let _ = chat.update(cx, |chat, cx| {
                chat.cache.take();
                if let Err(error) = result {
                    chat.fail(error.into(), cx);
                } else if chat.projection().cursor() != cursor {
                    chat.persist_projection(cx);
                }
                cx.notify();
            });
        }));
    }
}

#[cfg(test)]
mod tests;
