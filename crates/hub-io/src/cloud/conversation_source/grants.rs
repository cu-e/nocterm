//! Handing the runner of a chat a lease of the provider key, without a window (ADR 0126).
//!
//! The lease is an ordinary staged command, so a crash or lost acknowledgement resolves
//! through the same outbox as a prompt. The control revision is the one the source last
//! observed; a runner that moved on refuses the command and the next renewal sends a fresh one.
use super::CloudConversationSource;
use futures::future::BoxFuture;
use nocterm_cloud_model::projection::{
    ConversationIntent, ProjectionError, ProviderAccess,
    ports::{
        CapturedConversationSource, CommandDelivery, ConversationCoordinates, ConversationWork,
    },
};
use nocterm_core::allocation::{Allocated, AllocationLease};
use std::sync::Arc;

/// Gives the source the work allowance a command of the stated size needs.
pub(super) type ReserveWork =
    dyn Fn(usize) -> Result<AllocationLease, ProjectionError> + Send + Sync;

impl CloudConversationSource {
    /// Stages and delivers a lease of the provider key for this chat. A refusal by the runner
    /// is [`ProjectionError::Unavailable`], so the caller treats the lease as not held.
    pub fn grant_provider(
        self: &Arc<Self>,
        access: ProviderAccess,
        reserve: Arc<ReserveWork>,
    ) -> BoxFuture<'static, Result<(), ProjectionError>> {
        let source = self.clone();
        Box::pin(async move {
            source.check_current()?;
            let coordinates = ConversationCoordinates {
                binding: source.core.binding.clone(),
                run: None,
                control: source.core.marker()?.control,
            };
            let command = source.prepare(
                coordinates,
                ConversationIntent::GrantProvider(Box::new(access)),
            )?;
            let bytes = source
                .allocation_requirements(ConversationWork::Stage { command: &command })?
                .bytes();
            let work = reserve(bytes)?;
            let id = command.id;
            let staged = Arc::new(Allocated::new(command, work.clone()));
            source.stage(staged, work.clone()).await?;
            match source.deliver(id, work).await? {
                CommandDelivery::Delivered | CommandDelivery::Accepted => Ok(()),
                CommandDelivery::Pending | CommandDelivery::NotFound => {
                    Err(ProjectionError::OutcomeUnknown)
                }
                CommandDelivery::Rejected => Err(ProjectionError::Unavailable),
            }
        })
    }
}
