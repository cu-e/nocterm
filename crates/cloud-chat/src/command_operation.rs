//! An owner-held operation stages the exact original ID/body before transport.
use crate::{
    CloudChat,
    pending::{PendingCommandState, RetainedCommand},
};
use gpui::{Context, Task};
use nocterm_cloud_model::projection::{
    ProjectionError,
    ports::{CapturedConversationSource, CommandDelivery},
};
use nocterm_core::allocation::AllocationLease;
use std::sync::Arc;

pub(crate) struct CommandOperation {
    source: Arc<dyn CapturedConversationSource>,
    retained: Arc<RetainedCommand>,
    work: AllocationLease,
    retry: bool,
}
impl CommandOperation {
    pub(crate) fn new(
        source: Arc<dyn CapturedConversationSource>,
        retained: Arc<RetainedCommand>,
        work: AllocationLease,
        retry: bool,
    ) -> Self {
        Self {
            source,
            retained,
            work,
            retry,
        }
    }
    pub(crate) fn start(self, cx: &mut Context<CloudChat>) -> Task<()> {
        cx.spawn(async move |chat, cx| {
            let id = self.retained.value().id;
            let result = self.drive().await;
            let _ = chat.update(cx, |chat, cx| {
                if matches!(result, Err(ProjectionError::Stale)) {
                    chat.fail(ProjectionError::Stale.into(), cx);
                    return;
                }
                if let Err(error) = &result {
                    chat.record_error(error.clone().into());
                }
                chat.finish_command(id, result, cx);
            });
        })
    }
    async fn drive(&self) -> Result<PendingCommandState, ProjectionError> {
        self.source.check_current()?;
        let command = self.retained.clone();
        if self.retry {
            let queried = self.source.query(command.value().id).await;
            self.source.check_current()?;
            match queried? {
                CommandDelivery::NotFound => {}
                delivery => return delivered(delivery),
            }
        }
        self.source.check_current()?;
        let staged = self.source.stage(command.clone(), self.work.clone()).await;
        self.source.check_current()?;
        staged?;
        let delivery = self.source.deliver(command.value().id).await;
        self.source.check_current()?;
        delivered(delivery?)
    }
}
fn delivered(delivery: CommandDelivery) -> Result<PendingCommandState, ProjectionError> {
    match delivery {
        CommandDelivery::NotFound => Err(ProjectionError::OutcomeUnknown),
        CommandDelivery::Pending => Ok(PendingCommandState::Pending),
        CommandDelivery::Accepted => Ok(PendingCommandState::Accepted),
        CommandDelivery::Rejected => Ok(PendingCommandState::Rejected),
    }
}
