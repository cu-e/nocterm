//! A subscription owns its original capture before first poll and through cache I/O.
//! A poll nobody answered is retried with a growing delay; any other failure ends it.
use crate::{CloudChat, CloudChatError};
use gpui::{Context, Task};
use nocterm_cloud_model::projection::{ProjectionError, ports::CapturedConversationSource};
use std::{sync::Arc, time::Duration};

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
                let published = chat.update(cx, |chat, cx| match result {
                    Ok(nocterm_cloud_model::projection::ports::ConversationReplay::Page(batch)) => {
                        chat.accept_page(*batch, cx)
                    }
                    Ok(nocterm_cloud_model::projection::ports::ConversationReplay::Readmission) => {
                        Ok(())
                    }
                    Err(error) => Err(CloudChatError::from(error)),
                });
                match published {
                    Ok(Ok(())) => retry = FIRST_RETRY,
                    // The hub or the runner did not answer (a sleeping seat answers no head
                    // probe): the chat is not over, only out of contact.
                    Ok(Err(error @ CloudChatError::Source(ProjectionError::Unavailable))) => {
                        if chat
                            .update(cx, |chat, cx| chat.replay_interrupted(error, cx))
                            .is_err()
                        {
                            return;
                        }
                        cx.background_executor().timer(retry).await;
                        retry = (retry * 2).min(MAX_RETRY);
                    }
                    Ok(Err(error)) => {
                        let _ = chat.update(cx, |chat, cx| chat.fail(error, cx));
                        return;
                    }
                    Err(_) => return,
                }
            }
        })
    }
}
