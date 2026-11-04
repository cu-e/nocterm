//! The conversation source of a managed chat, which wakes the agent's seat before the runner is
//! needed.
//!
//! A seat sleeps after its idle time even while a window shows its chat. The hub stores a
//! command for a sleeping seat, but only a running runner admits it and answers head probes, so
//! a prompt would wait forever and every receipt query of a retry would go unanswered. Waking is
//! idempotent per chat on the hub (`start_instance_for_chat`, ADR 0122 §8): a running seat only
//! receives access for the chat again. Replay never wakes a seat; looking at a chat costs nothing.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use futures::future::BoxFuture;
use nocterm_cloud_agents_model::AgentId;
use nocterm_cloud_model::identity::CommandId;
use nocterm_cloud_model::projection::{
    ConversationIntent, ConversationProjection, ProjectionBinding, ProjectionCursor,
    ProjectionError,
    ports::{
        AllocationRequirements, CapturedConversationSource, CommandDelivery, ConversationCommand,
        ConversationCoordinates, ConversationReplay, ConversationWork,
    },
};
use nocterm_core::allocation::{Allocated, AllocationLease};
use tokio::runtime::Handle;

use super::managed;
use crate::devices::Captured;

/// Stops remembered so that delivering one does not wake a seat; a forgotten one only wakes it.
const REMEMBERED_STOPS: usize = 32;

/// Wakes the seat that hosts one chat.
pub(super) trait SeatWake: Send + Sync {
    fn wake(&self) -> BoxFuture<'static, Result<(), ProjectionError>>;
}

/// The seat of a managed agent on the hub of the captured account.
pub(super) struct ManagedSeat {
    handle: Handle,
    captured: Arc<Captured>,
    agent: AgentId,
    runner: [u8; 16],
    chat: [u8; 16],
}

impl ManagedSeat {
    pub(super) fn new(
        handle: Handle,
        captured: Arc<Captured>,
        agent: AgentId,
        runner: [u8; 16],
        chat: [u8; 16],
    ) -> Self {
        Self {
            handle,
            captured,
            agent,
            runner,
            chat,
        }
    }
}

impl SeatWake for ManagedSeat {
    fn wake(&self) -> BoxFuture<'static, Result<(), ProjectionError>> {
        let (captured, agent, runner, chat) = (
            self.captured.clone(),
            self.agent.clone(),
            self.runner,
            self.chat,
        );
        let task = crate::task::spawn(&self.handle, async move {
            managed::start_for_chat(&captured, &agent, Some(runner), chat).await
        });
        Box::pin(async move {
            match task.await {
                Some(Ok(_)) => Ok(()),
                Some(Err(error)) => {
                    tracing::warn!(error = %nocterm_core::error::chain(&error), "the seat of a Cloud chat could not be woken");
                    // The command stays staged: retrying it wakes the seat again.
                    Err(ProjectionError::Unavailable)
                }
                None => Err(ProjectionError::Unavailable),
            }
        })
    }
}

/// A source that wakes its seat before it delivers or queries a command the runner must admit.
pub(super) struct WakingSource {
    inner: Arc<dyn CapturedConversationSource>,
    seat: Arc<dyn SeatWake>,
    stops: Mutex<VecDeque<CommandId>>,
}

impl WakingSource {
    pub(super) fn wrap(
        inner: Arc<dyn CapturedConversationSource>,
        seat: Arc<dyn SeatWake>,
    ) -> Arc<dyn CapturedConversationSource> {
        Arc::new(Self {
            inner,
            seat,
            stops: Mutex::new(VecDeque::with_capacity(REMEMBERED_STOPS)),
        })
    }

    /// Stopping needs no runner that sleeps: a sleeping seat runs nothing.
    fn needs_runner(&self, id: CommandId) -> bool {
        !self
            .stops
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&id)
    }

    /// Runs `operation` once the seat is awake. The operation is created only then: the native
    /// source starts its work as soon as it is asked.
    fn after_wake(
        &self,
        id: CommandId,
        operation: impl FnOnce(
            &dyn CapturedConversationSource,
        ) -> BoxFuture<'static, Result<CommandDelivery, ProjectionError>>
        + Send
        + 'static,
    ) -> BoxFuture<'static, Result<CommandDelivery, ProjectionError>> {
        if !self.needs_runner(id) {
            return operation(&*self.inner);
        }
        let wake = self.seat.wake();
        let inner = self.inner.clone();
        Box::pin(async move {
            wake.await?;
            inner.check_current()?;
            operation(&*inner).await
        })
    }
}

impl CapturedConversationSource for WakingSource {
    fn allocation_requirements(
        &self,
        work: ConversationWork<'_>,
    ) -> Result<AllocationRequirements, ProjectionError> {
        self.inner.allocation_requirements(work)
    }
    fn supports_prompt(&self) -> bool {
        self.inner.supports_prompt()
    }
    fn binding(&self) -> &ProjectionBinding {
        self.inner.binding()
    }
    fn check_current(&self) -> Result<(), ProjectionError> {
        self.inner.check_current()
    }
    fn ended(&self) -> BoxFuture<'static, ()> {
        self.inner.ended()
    }
    fn with_current(&self, publish: &mut dyn FnMut()) -> Result<(), ProjectionError> {
        self.inner.with_current(publish)
    }
    fn next(
        &self,
        after: ProjectionCursor,
        allocation: AllocationLease,
    ) -> BoxFuture<'static, Result<ConversationReplay, ProjectionError>> {
        self.inner.next(after, allocation)
    }
    fn prepare(
        &self,
        coordinates: ConversationCoordinates,
        action: ConversationIntent,
    ) -> Result<ConversationCommand, ProjectionError> {
        let stop = matches!(action, ConversationIntent::Stop);
        let command = self.inner.prepare(coordinates, action)?;
        if stop {
            let mut stops = self.stops.lock().unwrap_or_else(PoisonError::into_inner);
            if stops.len() == REMEMBERED_STOPS {
                stops.pop_front();
            }
            stops.push_back(command.id);
        }
        Ok(command)
    }
    fn stage(
        &self,
        command: Arc<Allocated<ConversationCommand>>,
        work: AllocationLease,
    ) -> BoxFuture<'static, Result<(), ProjectionError>> {
        self.inner.stage(command, work)
    }
    fn deliver(
        &self,
        id: CommandId,
        work: AllocationLease,
    ) -> BoxFuture<'static, Result<CommandDelivery, ProjectionError>> {
        self.after_wake(id, move |inner| inner.deliver(id, work))
    }
    fn query(
        &self,
        id: CommandId,
        work: AllocationLease,
    ) -> BoxFuture<'static, Result<CommandDelivery, ProjectionError>> {
        self.after_wake(id, move |inner| inner.query(id, work))
    }
    fn cache(
        &self,
        projection: Arc<Allocated<ConversationProjection>>,
        work: AllocationLease,
    ) -> BoxFuture<'static, Result<(), ProjectionError>> {
        self.inner.cache(projection, work)
    }
}

#[cfg(test)]
mod tests;
