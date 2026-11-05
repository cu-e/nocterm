//! A subscription owns its original capture before first poll and through cache I/O.
//! A poll nobody answered is retried with a growing delay; any other failure ends it.
use crate::{CloudChat, CloudChatError};
use gpui::{AsyncApp, Context, Task, WeakEntity};
use nocterm_cloud_model::projection::{ProjectionError, ports::CapturedConversationSource};
use std::{sync::Arc, time::Duration};

use nocterm_cloud_model::{
    identity::CommandId,
    projection::{ConversationProjection, ProjectionBatch},
};
use nocterm_core::allocation::{Allocated, AllocationLease};

pub(crate) struct PreparedPage {
    original: Arc<Allocated<ConversationProjection>>,
    batch: Allocated<ProjectionBatch>,
    allocation: AllocationLease,
}
pub(crate) struct ReducedPage {
    pub(super) original: Arc<Allocated<ConversationProjection>>,
    pub(super) next: Arc<Allocated<ConversationProjection>>,
    pub(super) accepted: Vec<CommandId>,
}
impl PreparedPage {
    pub(super) fn new(
        original: Arc<Allocated<ConversationProjection>>,
        batch: Allocated<ProjectionBatch>,
        allocation: AllocationLease,
    ) -> Self {
        Self {
            original,
            batch,
            allocation,
        }
    }
    fn reduce(self) -> Result<ReducedPage, ProjectionError> {
        let (next, accepted) = self.original.value().advanced(self.batch.value())?;
        if next.resident_bytes() > self.allocation.bytes() {
            return Err(ProjectionError::Capacity);
        }
        Ok(ReducedPage {
            original: self.original,
            next: Arc::new(Allocated::new(next, self.allocation)),
            accepted,
        })
    }
}

/// The first wait after an unanswered poll, doubled up to [`MAX_RETRY`].
pub(crate) const FIRST_RETRY: Duration = Duration::from_secs(1);
const MAX_RETRY: Duration = Duration::from_secs(30);

pub(crate) struct ReplaySubscription(Arc<dyn CapturedConversationSource>);
impl ReplaySubscription {
    pub(crate) fn new(source: Arc<dyn CapturedConversationSource>) -> Self {
        Self(source)
    }
    pub(crate) fn start(self, cx: &mut Context<CloudChat>) -> Task<()> {
        cx.spawn(async move |chat, cx| {
            let mut retry = FIRST_RETRY;
            loop {
                if let Err(error) = self.0.check_current() {
                    let _ = chat.update(cx, |chat, cx| chat.fail(error.into(), cx));
                    return;
                }
                let prepared = chat.update(cx, |chat, cx| chat.prepare_replay(cx));
                let (cursor, allocation) = match prepared {
                    Ok(Ok(value)) => value,
                    Ok(Err(error)) => {
                        let _ = chat.update(cx, |chat, cx| chat.fail(error, cx));
                        return;
                    }
                    Err(_) => return,
                };
                let result = self.0.next(cursor, allocation).await;
                let result = self.0.check_current().and(result);
                let published = match result {
                    Ok(nocterm_cloud_model::projection::ports::ConversationReplay::Page(batch)) => {
                        reduce_page(&self.0, &chat, cx, *batch).await
                    }
                    Ok(nocterm_cloud_model::projection::ports::ConversationReplay::Readmission) => {
                        Ok(())
                    }
                    Err(error) => Err(CloudChatError::from(error)),
                };
                match published {
                    Ok(()) => retry = FIRST_RETRY,
                    // The hub or the runner did not answer (a sleeping seat answers no head
                    // probe): the chat is not over, only out of contact.
                    Err(error @ CloudChatError::Source(ProjectionError::Unavailable)) => {
                        if chat
                            .update(cx, |chat, cx| chat.replay_interrupted(error, cx))
                            .is_err()
                        {
                            return;
                        }
                        cx.background_executor().timer(retry).await;
                        retry = (retry * 2).min(MAX_RETRY);
                    }
                    Err(error) => {
                        let _ = chat.update(cx, |chat, cx| chat.fail(error, cx));
                        return;
                    }
                }
            }
        })
    }
}

async fn reduce_page(
    source: &Arc<dyn CapturedConversationSource>,
    chat: &WeakEntity<CloudChat>,
    cx: &mut AsyncApp,
    batch: Allocated<ProjectionBatch>,
) -> Result<(), CloudChatError> {
    let (prepared, payload) = chat
        .update(cx, |chat, cx| chat.prepare_page(batch, cx))
        .map_err(|_| CloudChatError::Closed)??;
    let result = cx
        .background_executor()
        .spawn(async move { prepared.reduce() })
        .await;
    let page = source.check_current().and(result)?;
    chat.update(cx, |chat, cx| chat.accept_page(page, payload, cx))
        .map_err(|_| CloudChatError::Closed)?
}
